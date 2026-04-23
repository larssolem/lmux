//! In-memory anchor registry.
//!
//! This is the pure-data layer of Epic 6 — state transitions only, no OS
//! effects. The eventual live-process impl will wrap this registry in a
//! struct that also drives SIGSTOP/SIGCONT and scrollback rings.

use std::collections::HashMap;

use thiserror::Error;
use uuid::Uuid;

use crate::{Anchor, AnchorState};

#[derive(Debug, Error)]
pub enum RegistryError {
    #[error("anchor {0} not found")]
    NotFound(Uuid),
    #[error("invalid transition: {anchor} is {from:?}, cannot -> {to:?}")]
    InvalidTransition {
        anchor: Uuid,
        from: AnchorState,
        to: AnchorState,
    },
}

#[derive(Debug, Default)]
pub struct AnchorRegistry {
    anchors: HashMap<Uuid, Anchor>,
}

impl AnchorRegistry {
    pub fn insert(&mut self, anchor: Anchor) -> Uuid {
        let id = anchor.id;
        self.anchors.insert(id, anchor);
        id
    }

    pub fn get(&self, id: Uuid) -> Option<&Anchor> {
        self.anchors.get(&id)
    }

    pub fn iter(&self) -> impl Iterator<Item = &Anchor> {
        self.anchors.values()
    }

    pub fn remove(&mut self, id: Uuid) -> Result<Anchor, RegistryError> {
        self.anchors.remove(&id).ok_or(RegistryError::NotFound(id))
    }

    /// Live → Paused. Valid only from Live.
    pub fn pause(&mut self, id: Uuid) -> Result<&Anchor, RegistryError> {
        self.transition(id, AnchorState::Live, AnchorState::Paused)
    }

    /// Paused → Live. Valid only from Paused.
    pub fn resume(&mut self, id: Uuid) -> Result<&Anchor, RegistryError> {
        self.transition(id, AnchorState::Paused, AnchorState::Live)
    }

    /// Live or Paused → Hidden. Drops the pane_id binding.
    pub fn hide(&mut self, id: Uuid) -> Result<&Anchor, RegistryError> {
        let a = self
            .anchors
            .get_mut(&id)
            .ok_or(RegistryError::NotFound(id))?;
        if !matches!(a.state, AnchorState::Live | AnchorState::Paused) {
            return Err(RegistryError::InvalidTransition {
                anchor: id,
                from: a.state,
                to: AnchorState::Hidden,
            });
        }
        a.state = AnchorState::Hidden;
        a.pane_id = None;
        Ok(a)
    }

    /// Hidden → Live, attached to `pane_id`.
    pub fn reattach(&mut self, id: Uuid, pane_id: Uuid) -> Result<&Anchor, RegistryError> {
        let a = self
            .anchors
            .get_mut(&id)
            .ok_or(RegistryError::NotFound(id))?;
        if a.state != AnchorState::Hidden {
            return Err(RegistryError::InvalidTransition {
                anchor: id,
                from: a.state,
                to: AnchorState::Live,
            });
        }
        a.state = AnchorState::Live;
        a.pane_id = Some(pane_id);
        Ok(a)
    }

    /// Toggle a soft-hide state: flips `state` between `Hidden` and `Live`
    /// WITHOUT dropping the pane_id binding. v0.2's widget-level hide uses
    /// this variant because the backing Pane + PTY stay alive — only the
    /// GTK widget is detached. Invalid from `Paused`/`Dead`.
    pub fn set_hidden(&mut self, id: Uuid, hidden: bool) -> Result<&Anchor, RegistryError> {
        let a = self
            .anchors
            .get_mut(&id)
            .ok_or(RegistryError::NotFound(id))?;
        let to = if hidden {
            AnchorState::Hidden
        } else {
            AnchorState::Live
        };
        if !matches!(a.state, AnchorState::Live | AnchorState::Hidden) {
            return Err(RegistryError::InvalidTransition {
                anchor: id,
                from: a.state,
                to,
            });
        }
        a.state = to;
        Ok(a)
    }

    /// Mark an anchor as Dead. Valid from any state (process can die
    /// while Paused or Hidden).
    pub fn mark_dead(&mut self, id: Uuid) -> Result<&Anchor, RegistryError> {
        let a = self
            .anchors
            .get_mut(&id)
            .ok_or(RegistryError::NotFound(id))?;
        a.state = AnchorState::Dead;
        Ok(a)
    }

    /// Set or clear the display name. Empty strings are stored as `None`
    /// so the sidebar falls back to the derived label.
    pub fn rename(&mut self, id: Uuid, name: Option<String>) -> Result<&Anchor, RegistryError> {
        let a = self
            .anchors
            .get_mut(&id)
            .ok_or(RegistryError::NotFound(id))?;
        a.name = name.and_then(|s| {
            let trimmed = s.trim().to_string();
            if trimmed.is_empty() {
                None
            } else {
                Some(trimmed)
            }
        });
        Ok(a)
    }

    /// Move the anchor to a named group (or to `None` for ungrouped).
    pub fn set_group(&mut self, id: Uuid, group: Option<String>) -> Result<&Anchor, RegistryError> {
        let a = self
            .anchors
            .get_mut(&id)
            .ok_or(RegistryError::NotFound(id))?;
        a.group = group.and_then(|s| {
            let trimmed = s.trim().to_string();
            if trimmed.is_empty() {
                None
            } else {
                Some(trimmed)
            }
        });
        Ok(a)
    }

    /// Assign a manual sort key. Lower sorts first. `None` resets to default
    /// (treated as 0 during rendering).
    pub fn set_sort_key(
        &mut self,
        id: Uuid,
        sort_key: Option<i64>,
    ) -> Result<&Anchor, RegistryError> {
        let a = self
            .anchors
            .get_mut(&id)
            .ok_or(RegistryError::NotFound(id))?;
        a.sort_key = sort_key;
        Ok(a)
    }

    /// Anchors grouped and sorted for sidebar rendering.
    ///
    /// Groups are sorted alphabetically with `None` ("Ungrouped") last.
    /// Within a group: sort_key ascending, then display label ascending.
    pub fn grouped_for_sidebar(&self) -> Vec<(Option<String>, Vec<&Anchor>)> {
        let mut by_group: HashMap<Option<String>, Vec<&Anchor>> = HashMap::new();
        for a in self.anchors.values() {
            by_group.entry(a.group.clone()).or_default().push(a);
        }
        for bucket in by_group.values_mut() {
            bucket.sort_by(|a, b| {
                let ak = a.sort_key.unwrap_or(0);
                let bk = b.sort_key.unwrap_or(0);
                ak.cmp(&bk)
                    .then_with(|| a.display_label().cmp(b.display_label()))
            });
        }
        let mut out: Vec<_> = by_group.into_iter().collect();
        out.sort_by(|(a, _), (b, _)| match (a, b) {
            (Some(x), Some(y)) => x.cmp(y),
            (Some(_), None) => std::cmp::Ordering::Less,
            (None, Some(_)) => std::cmp::Ordering::Greater,
            (None, None) => std::cmp::Ordering::Equal,
        });
        out
    }

    fn transition(
        &mut self,
        id: Uuid,
        from: AnchorState,
        to: AnchorState,
    ) -> Result<&Anchor, RegistryError> {
        let a = self
            .anchors
            .get_mut(&id)
            .ok_or(RegistryError::NotFound(id))?;
        if a.state != from {
            return Err(RegistryError::InvalidTransition {
                anchor: id,
                from: a.state,
                to,
            });
        }
        a.state = to;
        Ok(a)
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::*;

    fn anchor() -> Anchor {
        Anchor::new_manual(Uuid::new_v4(), vec!["bash".into()], "/tmp".into())
    }

    #[test]
    fn insert_and_get() {
        let mut reg = AnchorRegistry::default();
        let a = anchor();
        let id = a.id;
        reg.insert(a);
        assert!(reg.get(id).is_some());
    }

    #[test]
    fn pause_then_resume_roundtrips() {
        let mut reg = AnchorRegistry::default();
        let id = reg.insert(anchor());
        reg.pause(id).unwrap();
        assert_eq!(reg.get(id).unwrap().state, AnchorState::Paused);
        reg.resume(id).unwrap();
        assert_eq!(reg.get(id).unwrap().state, AnchorState::Live);
    }

    #[test]
    fn resume_from_live_rejected() {
        let mut reg = AnchorRegistry::default();
        let id = reg.insert(anchor());
        assert!(matches!(
            reg.resume(id),
            Err(RegistryError::InvalidTransition { .. })
        ));
    }

    #[test]
    fn hide_drops_pane_binding() {
        let mut reg = AnchorRegistry::default();
        let id = reg.insert(anchor());
        reg.hide(id).unwrap();
        let a = reg.get(id).unwrap();
        assert_eq!(a.state, AnchorState::Hidden);
        assert!(a.pane_id.is_none());
    }

    #[test]
    fn reattach_requires_hidden() {
        let mut reg = AnchorRegistry::default();
        let id = reg.insert(anchor());
        let pane = Uuid::new_v4();
        assert!(matches!(
            reg.reattach(id, pane),
            Err(RegistryError::InvalidTransition { .. })
        ));
        reg.hide(id).unwrap();
        reg.reattach(id, pane).unwrap();
        let a = reg.get(id).unwrap();
        assert_eq!(a.state, AnchorState::Live);
        assert_eq!(a.pane_id, Some(pane));
    }

    #[test]
    fn set_hidden_preserves_pane_binding() {
        let mut reg = AnchorRegistry::default();
        let id = reg.insert(anchor());
        let original_pane = reg.get(id).unwrap().pane_id;
        assert!(original_pane.is_some());
        reg.set_hidden(id, true).unwrap();
        let a = reg.get(id).unwrap();
        assert_eq!(a.state, AnchorState::Hidden);
        assert_eq!(a.pane_id, original_pane, "soft-hide must keep the binding");
        reg.set_hidden(id, false).unwrap();
        assert_eq!(reg.get(id).unwrap().state, AnchorState::Live);
    }

    #[test]
    fn set_hidden_rejected_from_paused() {
        let mut reg = AnchorRegistry::default();
        let id = reg.insert(anchor());
        reg.pause(id).unwrap();
        assert!(matches!(
            reg.set_hidden(id, true),
            Err(RegistryError::InvalidTransition { .. })
        ));
    }

    #[test]
    fn mark_dead_from_any_state() {
        let mut reg = AnchorRegistry::default();
        let id = reg.insert(anchor());
        reg.pause(id).unwrap();
        reg.mark_dead(id).unwrap();
        assert_eq!(reg.get(id).unwrap().state, AnchorState::Dead);
    }

    #[test]
    fn rename_sets_and_trims() {
        let mut reg = AnchorRegistry::default();
        let id = reg.insert(anchor());
        reg.rename(id, Some("  Build watcher  ".into())).unwrap();
        assert_eq!(reg.get(id).unwrap().name.as_deref(), Some("Build watcher"));
        reg.rename(id, Some("   ".into())).unwrap();
        assert!(reg.get(id).unwrap().name.is_none());
        reg.rename(id, Some("x".into())).unwrap();
        reg.rename(id, None).unwrap();
        assert!(reg.get(id).unwrap().name.is_none());
    }

    #[test]
    fn display_label_falls_back_to_argv() {
        let mut reg = AnchorRegistry::default();
        let id = reg.insert(anchor());
        assert_eq!(reg.get(id).unwrap().display_label(), "bash");
        reg.rename(id, Some("Dev server".into())).unwrap();
        assert_eq!(reg.get(id).unwrap().display_label(), "Dev server");
    }

    #[test]
    fn grouped_for_sidebar_orders_groups_and_within() {
        let mut reg = AnchorRegistry::default();
        let a1 = reg.insert(anchor());
        let a2 = reg.insert(anchor());
        let a3 = reg.insert(anchor());
        let a4 = reg.insert(anchor());

        reg.set_group(a1, Some("Services".into())).unwrap();
        reg.set_group(a2, Some("Services".into())).unwrap();
        reg.set_group(a3, Some("Build".into())).unwrap();
        // a4 left ungrouped

        reg.rename(a1, Some("beta".into())).unwrap();
        reg.rename(a2, Some("alpha".into())).unwrap();
        reg.set_sort_key(a1, Some(-1)).unwrap(); // beta floats above alpha via sort_key

        let grouped = reg.grouped_for_sidebar();
        assert_eq!(grouped.len(), 3);
        assert_eq!(grouped[0].0.as_deref(), Some("Build"));
        assert_eq!(grouped[1].0.as_deref(), Some("Services"));
        assert_eq!(grouped[2].0, None);

        // Within Services: beta (sort_key=-1) before alpha (sort_key default 0).
        let services = &grouped[1].1;
        assert_eq!(services[0].id, a1);
        assert_eq!(services[1].id, a2);

        // Ungrouped bucket has a4.
        assert_eq!(grouped[2].1.len(), 1);
        assert_eq!(grouped[2].1[0].id, a4);
    }

    #[test]
    fn not_found_surfaces() {
        let mut reg = AnchorRegistry::default();
        let missing = Uuid::new_v4();
        assert!(matches!(
            reg.pause(missing),
            Err(RegistryError::NotFound(_))
        ));
    }
}

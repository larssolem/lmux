use std::collections::{BTreeSet, HashMap};
use std::time::{SystemTime, UNIX_EPOCH};

use lmux_bus::{AgentIdentity, AgentPaneStatus, BusError, GrantDecision, GrantRequest, GrantScope};
use uuid::Uuid;

use crate::layout::PaneId;

#[derive(Clone)]
pub(crate) struct AgentPaneMetadata {
    pub(crate) agent: AgentIdentity,
    pub(crate) purpose: Option<String>,
}

#[derive(Clone)]
pub(crate) struct GrantRecord {
    pub(crate) request: GrantRequest,
    pub(crate) state: GrantState,
}

#[derive(Clone)]
pub(crate) enum GrantState {
    Pending,
    AllowedOnce,
    AllowedUntil(u64),
    Denied,
    Revoked,
}

#[derive(Clone)]
pub(crate) struct GrantView {
    pub(crate) grant_id: Uuid,
    pub(crate) requester: AgentIdentity,
    pub(crate) scope: GrantScope,
    pub(crate) reason: Option<String>,
    pub(crate) pending: bool,
    pub(crate) active: bool,
}

pub(crate) fn agent_status_for_anchor(
    anchor_pane: PaneId,
    pane_agents: &HashMap<PaneId, AgentPaneMetadata>,
    pane_workspace: &HashMap<PaneId, PaneId>,
    pane_uuids: &HashMap<PaneId, Uuid>,
    pane_titles: &HashMap<PaneId, lmux_bus::PaneTitle>,
) -> Vec<AgentPaneStatus> {
    pane_agents
        .iter()
        .filter_map(|(agent_pane_id, meta)| {
            let owner = pane_workspace.get(agent_pane_id).copied();
            if *agent_pane_id != anchor_pane && owner != Some(anchor_pane) {
                return None;
            }
            let pane_uuid = pane_uuids.get(agent_pane_id).copied()?;
            let title = pane_titles
                .get(agent_pane_id)
                .map(|pane_title| pane_title.title.clone());
            Some(AgentPaneStatus {
                pane_id: pane_uuid,
                agent: meta.agent.clone(),
                title,
                purpose: meta.purpose.clone(),
            })
        })
        .collect()
}

pub(crate) fn pending_grants_for_anchor(
    grants: &HashMap<Uuid, GrantRecord>,
    anchor_id: Uuid,
) -> u32 {
    grants
        .values()
        .filter(|record| {
            record.request.target_anchor == Some(anchor_id)
                && matches!(record.state, GrantState::Pending)
        })
        .count() as u32
}

pub(crate) fn active_grants_for_anchor(
    grants: &HashMap<Uuid, GrantRecord>,
    anchor_id: Uuid,
) -> u32 {
    let now = unix_seconds();
    grants
        .values()
        .filter(|record| {
            if record.request.target_anchor != Some(anchor_id) {
                return false;
            }
            match record.state {
                GrantState::AllowedOnce => true,
                GrantState::AllowedUntil(expires_at) => expires_at > now,
                _ => false,
            }
        })
        .count() as u32
}

pub(crate) fn grant_views_for_anchor(
    grants: &HashMap<Uuid, GrantRecord>,
    anchor_id: Uuid,
) -> Vec<GrantView> {
    let now = unix_seconds();
    grants
        .values()
        .filter(|record| record.request.target_anchor == Some(anchor_id))
        .filter_map(|record| {
            let (pending, active) = match record.state {
                GrantState::Pending => (true, false),
                GrantState::AllowedOnce => (false, true),
                GrantState::AllowedUntil(expires_at) if expires_at > now => (false, true),
                _ => return None,
            };
            Some(GrantView {
                grant_id: record.request.grant_id,
                requester: record.request.requester.clone(),
                scope: record.request.scope,
                reason: record.request.reason.clone(),
                pending,
                active,
            })
        })
        .collect()
}

pub(crate) fn register_grant_request(
    grants: &mut HashMap<Uuid, GrantRecord>,
    request: GrantRequest,
) -> GrantRequest {
    grants.insert(
        request.grant_id,
        GrantRecord {
            request: request.clone(),
            state: GrantState::Pending,
        },
    );
    request
}

pub(crate) fn decide_grant(
    grants: &mut HashMap<Uuid, GrantRecord>,
    grant_id: Uuid,
    decision: GrantDecision,
) -> Result<(), BusError> {
    let record = grants
        .get_mut(&grant_id)
        .ok_or_else(|| BusError::Domain(format!("grant.decide: unknown grant {grant_id}")))?;
    record.state = match decision {
        GrantDecision::AllowOnce => GrantState::AllowedOnce,
        GrantDecision::AllowUntil {
            expires_at_unix_seconds,
        } => GrantState::AllowedUntil(expires_at_unix_seconds),
        GrantDecision::Deny => GrantState::Denied,
        GrantDecision::Revoke => GrantState::Revoked,
    };
    Ok(())
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn authorize_pane_access(
    grants: &mut HashMap<Uuid, GrantRecord>,
    pane_agents: &HashMap<PaneId, AgentPaneMetadata>,
    anchors: &BTreeSet<PaneId>,
    pane_workspace: &HashMap<PaneId, PaneId>,
    pane_anchor_ids: &HashMap<PaneId, Uuid>,
    target_pane: PaneId,
    target_pane_uuid: Uuid,
    scope: GrantScope,
    agent: Option<&AgentIdentity>,
    target_window: Option<String>,
    reason: &str,
) -> Result<(), BusError> {
    let Some(agent) = agent else {
        return Ok(());
    };
    if pane_agents
        .get(&target_pane)
        .is_some_and(|meta| same_agent(&meta.agent, agent))
    {
        return Ok(());
    }

    let target_anchor_pane = owner_anchor(target_pane, anchors, pane_workspace);
    let target_anchor = target_anchor_pane.and_then(|pane| pane_anchor_ids.get(&pane).copied());
    let source_anchor_pane = first_agent_anchor(agent, pane_agents, anchors, pane_workspace);
    if target_anchor_pane.is_some() && target_anchor_pane == source_anchor_pane {
        return Ok(());
    }
    let source_anchor = source_anchor_pane.and_then(|pane| pane_anchor_ids.get(&pane).copied());

    if let Some(result) = existing_grant_decision(
        grants,
        agent,
        scope,
        target_anchor,
        target_pane_uuid,
        target_window.as_deref(),
        unix_seconds(),
    ) {
        return result;
    }

    let request = GrantRequest {
        grant_id: Uuid::new_v4(),
        requester: agent.clone(),
        scope,
        source_anchor,
        target_anchor,
        target_pane: Some(target_pane_uuid),
        target_window,
        reason: Some(reason.to_string()),
    };
    let grant_id = request.grant_id;
    register_grant_request(grants, request);
    Err(BusError::Unauthorized(format!(
        "grant required; request {grant_id} is pending"
    )))
}

fn same_agent(a: &AgentIdentity, b: &AgentIdentity) -> bool {
    a.id == b.id
}

fn owner_anchor(
    pane: PaneId,
    anchors: &BTreeSet<PaneId>,
    pane_workspace: &HashMap<PaneId, PaneId>,
) -> Option<PaneId> {
    pane_workspace
        .get(&pane)
        .copied()
        .or_else(|| anchors.contains(&pane).then_some(pane))
}

fn first_agent_anchor(
    agent: &AgentIdentity,
    pane_agents: &HashMap<PaneId, AgentPaneMetadata>,
    anchors: &BTreeSet<PaneId>,
    pane_workspace: &HashMap<PaneId, PaneId>,
) -> Option<PaneId> {
    pane_agents.iter().find_map(|(pane, meta)| {
        same_agent(&meta.agent, agent).then(|| owner_anchor(*pane, anchors, pane_workspace))?
    })
}

fn existing_grant_decision(
    grants: &mut HashMap<Uuid, GrantRecord>,
    agent: &AgentIdentity,
    scope: GrantScope,
    target_anchor: Option<Uuid>,
    target_pane: Uuid,
    target_window: Option<&str>,
    now: u64,
) -> Option<Result<(), BusError>> {
    let grant_id = grants.iter().find_map(|(grant_id, record)| {
        grant_matches(
            record,
            agent,
            scope,
            target_anchor,
            target_pane,
            target_window,
        )
        .then_some(*grant_id)
    })?;
    let record = grants.get_mut(&grant_id)?;
    Some(match record.state {
        GrantState::AllowedOnce => {
            record.state = GrantState::Revoked;
            Ok(())
        }
        GrantState::AllowedUntil(expires_at) if expires_at > now => Ok(()),
        GrantState::AllowedUntil(_) => Err(BusError::Unauthorized(format!(
            "grant {grant_id} expired for {scope:?}"
        ))),
        GrantState::Pending => Err(BusError::Unauthorized(format!(
            "grant {grant_id} is pending for {scope:?}"
        ))),
        GrantState::Denied => Err(BusError::GrantDenied(format!(
            "grant {grant_id} was denied for {scope:?}"
        ))),
        GrantState::Revoked => Err(BusError::GrantDenied(format!(
            "grant {grant_id} was revoked for {scope:?}"
        ))),
    })
}

fn grant_matches(
    record: &GrantRecord,
    agent: &AgentIdentity,
    scope: GrantScope,
    target_anchor: Option<Uuid>,
    target_pane: Uuid,
    target_window: Option<&str>,
) -> bool {
    same_agent(&record.request.requester, agent)
        && record.request.scope == scope
        && window_target_matches(record, target_window)
        && (record.request.target_pane == Some(target_pane)
            || (target_anchor.is_some() && record.request.target_anchor == target_anchor))
}

fn window_target_matches(record: &GrantRecord, target_window: Option<&str>) -> bool {
    match (record.request.target_window.as_deref(), target_window) {
        (None, None) => true,
        (Some(expected), Some(actual)) => expected == actual,
        _ => false,
    }
}

fn unix_seconds() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::*;

    fn agent(id: &str) -> AgentIdentity {
        AgentIdentity {
            id: id.into(),
            name: None,
        }
    }

    #[test]
    fn own_anchor_access_is_allowed() {
        let target_uuid = Uuid::new_v4();
        let anchor_uuid = Uuid::new_v4();
        let mut grants = HashMap::new();
        let mut agents = HashMap::new();
        agents.insert(
            2,
            AgentPaneMetadata {
                agent: agent("a"),
                purpose: None,
            },
        );
        let anchors = BTreeSet::from([1]);
        let workspace = HashMap::from([(2, 1), (3, 1)]);
        let anchor_ids = HashMap::from([(1, anchor_uuid)]);

        let result = authorize_pane_access(
            &mut grants,
            &agents,
            &anchors,
            &workspace,
            &anchor_ids,
            3,
            target_uuid,
            GrantScope::ReadOutput,
            Some(&agent("a")),
            None,
            "test",
        );

        assert!(result.is_ok());
        assert!(grants.is_empty());
    }

    #[test]
    fn cross_anchor_access_creates_pending_grant() {
        let source_anchor = Uuid::new_v4();
        let target_anchor = Uuid::new_v4();
        let target_pane = Uuid::new_v4();
        let mut grants = HashMap::new();
        let mut agents = HashMap::new();
        agents.insert(
            2,
            AgentPaneMetadata {
                agent: agent("a"),
                purpose: None,
            },
        );
        let anchors = BTreeSet::from([1, 4]);
        let workspace = HashMap::from([(2, 1), (5, 4)]);
        let anchor_ids = HashMap::from([(1, source_anchor), (4, target_anchor)]);

        let result = authorize_pane_access(
            &mut grants,
            &agents,
            &anchors,
            &workspace,
            &anchor_ids,
            5,
            target_pane,
            GrantScope::ReadOutput,
            Some(&agent("a")),
            None,
            "test",
        );

        assert!(matches!(result, Err(BusError::Unauthorized(_))));
        let request = &grants.values().next().unwrap().request;
        assert_eq!(request.source_anchor, Some(source_anchor));
        assert_eq!(request.target_anchor, Some(target_anchor));
        assert_eq!(request.target_pane, Some(target_pane));
    }

    #[test]
    fn deny_and_revoke_block_access() {
        let target_anchor = Uuid::new_v4();
        let target_pane = Uuid::new_v4();
        let agent = agent("a");
        let grant_id = Uuid::new_v4();
        let mut grants = HashMap::from([(
            grant_id,
            GrantRecord {
                request: GrantRequest {
                    grant_id,
                    requester: agent.clone(),
                    scope: GrantScope::SendInput,
                    source_anchor: None,
                    target_anchor: Some(target_anchor),
                    target_pane: Some(target_pane),
                    target_window: None,
                    reason: None,
                },
                state: GrantState::Denied,
            },
        )]);

        let denied = existing_grant_decision(
            &mut grants,
            &agent,
            GrantScope::SendInput,
            Some(target_anchor),
            target_pane,
            None,
            unix_seconds(),
        );
        assert!(matches!(denied, Some(Err(BusError::GrantDenied(_)))));

        grants.get_mut(&grant_id).unwrap().state = GrantState::Revoked;
        let revoked = existing_grant_decision(
            &mut grants,
            &agent,
            GrantScope::SendInput,
            Some(target_anchor),
            target_pane,
            None,
            unix_seconds(),
        );
        assert!(matches!(revoked, Some(Err(BusError::GrantDenied(_)))));
    }

    #[test]
    fn expired_timed_grant_blocks_access() {
        let target_pane = Uuid::new_v4();
        let agent = agent("a");
        let grant_id = Uuid::new_v4();
        let mut grants = HashMap::from([(
            grant_id,
            GrantRecord {
                request: GrantRequest {
                    grant_id,
                    requester: agent.clone(),
                    scope: GrantScope::ReadOutput,
                    source_anchor: None,
                    target_anchor: None,
                    target_pane: Some(target_pane),
                    target_window: None,
                    reason: None,
                },
                state: GrantState::AllowedUntil(1),
            },
        )]);

        let result = existing_grant_decision(
            &mut grants,
            &agent,
            GrantScope::ReadOutput,
            None,
            target_pane,
            None,
            unix_seconds(),
        );

        assert!(matches!(result, Some(Err(BusError::Unauthorized(_)))));
    }

    #[test]
    fn attach_window_scope_uses_grant_matching() {
        let target_anchor = Uuid::new_v4();
        let target_pane = Uuid::new_v4();
        let agent = agent("a");
        let grant_id = Uuid::new_v4();
        let mut grants = HashMap::from([(
            grant_id,
            GrantRecord {
                request: GrantRequest {
                    grant_id,
                    requester: agent.clone(),
                    scope: GrantScope::AttachWindow,
                    source_anchor: None,
                    target_anchor: Some(target_anchor),
                    target_pane: Some(target_pane),
                    target_window: Some("Window".into()),
                    reason: Some("attach Window".into()),
                },
                state: GrantState::AllowedOnce,
            },
        )]);

        let result = existing_grant_decision(
            &mut grants,
            &agent,
            GrantScope::AttachWindow,
            Some(target_anchor),
            target_pane,
            Some("Window"),
            unix_seconds(),
        );

        assert!(matches!(result, Some(Ok(()))));
        assert!(matches!(
            grants.get(&grant_id).map(|record| &record.state),
            Some(GrantState::Revoked)
        ));
    }

    #[test]
    fn attach_window_grants_are_candidate_specific() {
        let target_anchor = Uuid::new_v4();
        let target_pane = Uuid::new_v4();
        let agent = agent("a");
        let grant_id = Uuid::new_v4();
        let mut grants = HashMap::from([(
            grant_id,
            GrantRecord {
                request: GrantRequest {
                    grant_id,
                    requester: agent.clone(),
                    scope: GrantScope::AttachWindow,
                    source_anchor: None,
                    target_anchor: Some(target_anchor),
                    target_pane: Some(target_pane),
                    target_window: Some("Window A".into()),
                    reason: Some("attach Window A".into()),
                },
                state: GrantState::AllowedUntil(unix_seconds() + 60),
            },
        )]);

        let unrelated = existing_grant_decision(
            &mut grants,
            &agent,
            GrantScope::AttachWindow,
            Some(target_anchor),
            target_pane,
            Some("Window B"),
            unix_seconds(),
        );
        assert!(unrelated.is_none());

        let exact = existing_grant_decision(
            &mut grants,
            &agent,
            GrantScope::AttachWindow,
            Some(target_anchor),
            target_pane,
            Some("Window A"),
            unix_seconds(),
        );
        assert!(matches!(exact, Some(Ok(()))));
    }

    #[test]
    fn denied_and_expired_attach_window_grants_block_access() {
        let target_anchor = Uuid::new_v4();
        let target_pane = Uuid::new_v4();
        let agent = agent("a");
        let denied_grant_id = Uuid::new_v4();
        let expired_grant_id = Uuid::new_v4();
        let mut grants = HashMap::from([
            (
                denied_grant_id,
                GrantRecord {
                    request: GrantRequest {
                        grant_id: denied_grant_id,
                        requester: agent.clone(),
                        scope: GrantScope::AttachWindow,
                        source_anchor: None,
                        target_anchor: Some(target_anchor),
                        target_pane: Some(target_pane),
                        target_window: Some("Denied Window".into()),
                        reason: Some("attach Denied Window".into()),
                    },
                    state: GrantState::Denied,
                },
            ),
            (
                expired_grant_id,
                GrantRecord {
                    request: GrantRequest {
                        grant_id: expired_grant_id,
                        requester: agent.clone(),
                        scope: GrantScope::AttachWindow,
                        source_anchor: None,
                        target_anchor: Some(target_anchor),
                        target_pane: Some(target_pane),
                        target_window: Some("Expired Window".into()),
                        reason: Some("attach Expired Window".into()),
                    },
                    state: GrantState::AllowedUntil(1),
                },
            ),
        ]);

        let denied = existing_grant_decision(
            &mut grants,
            &agent,
            GrantScope::AttachWindow,
            Some(target_anchor),
            target_pane,
            Some("Denied Window"),
            unix_seconds(),
        );
        assert!(matches!(denied, Some(Err(BusError::GrantDenied(_)))));

        let expired = existing_grant_decision(
            &mut grants,
            &agent,
            GrantScope::AttachWindow,
            Some(target_anchor),
            target_pane,
            Some("Expired Window"),
            unix_seconds(),
        );
        assert!(matches!(expired, Some(Err(BusError::Unauthorized(_)))));
    }
}

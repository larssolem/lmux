//! Layout tree for splits. A tree whose leaves are [`PaneId`]s and whose
//! internal nodes are splits. The UI rebuilds a matching tree of
//! [`gtk4::Paned`] widgets from this structure.

pub type PaneId = u32;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Dir {
    /// Split along a horizontal line — children are stacked top/bottom.
    Horizontal,
    /// Split along a vertical line — children sit side by side.
    Vertical,
}

/// Which edge of a pane the user dropped a dragged pane onto. Used by
/// rearrange mode to translate a drop coordinate into a re-parent op:
/// `Top`/`Bottom` create a horizontal split; `Left`/`Right` create a
/// vertical split. Whether the dropped pane ends up as the `a` (top/left)
/// or `b` (bottom/right) child of the new split is dictated by the edge.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Edge {
    Top,
    Right,
    Bottom,
    Left,
}

impl Edge {
    /// Pick the closest edge of a `width × height` rect for the cursor
    /// position `(x, y)` in widget-local pixels. Falls back to whichever
    /// margin is smallest, so a center drop still resolves to an edge
    /// rather than no-op.
    pub fn from_xy(x: f64, y: f64, width: f64, height: f64) -> Self {
        let left = x;
        let right = (width - x).max(0.0);
        let top = y;
        let bottom = (height - y).max(0.0);
        let min = left.min(right).min(top).min(bottom);
        if min == top {
            Edge::Top
        } else if min == bottom {
            Edge::Bottom
        } else if min == left {
            Edge::Left
        } else {
            Edge::Right
        }
    }
}

#[derive(Debug, Clone)]
pub enum Layout {
    Leaf(PaneId),
    Split {
        dir: Dir,
        a: Box<Layout>,
        b: Box<Layout>,
        ratio: f64,
    },
}

impl Layout {
    /// Collect leaf ids in in-order traversal (left/top child first).
    pub fn leaves(&self) -> Vec<PaneId> {
        let mut out = Vec::new();
        self.collect_leaves(&mut out);
        out
    }

    fn collect_leaves(&self, out: &mut Vec<PaneId>) {
        match self {
            Layout::Leaf(id) => out.push(*id),
            Layout::Split { a, b, .. } => {
                a.collect_leaves(out);
                b.collect_leaves(out);
            }
        }
    }

    /// Replace the leaf with the given id by the result of `f(id)`, returning
    /// true on success. Used to splice new split nodes in place.
    pub fn replace_leaf<F>(&mut self, target: PaneId, f: F) -> bool
    where
        F: FnOnce(PaneId) -> Layout,
    {
        replace_leaf_with(self, target, &mut Some(f))
    }

    /// Remove the leaf `target`. If the leaf is one child of a Split, the other
    /// child replaces the Split. If `target` is the root (single-pane layout),
    /// returns false without modifying the tree.
    pub fn remove_leaf(&mut self, target: PaneId) -> bool {
        if matches!(self, Layout::Leaf(id) if *id == target) {
            return false;
        }
        remove_leaf_in(self, target)
    }

    /// Move `source` next to `target` along the given `edge`. Equivalent to
    /// `remove_leaf(source)` followed by replacing `target`'s leaf with a
    /// new split. Returns false (and leaves the tree unchanged) when:
    /// - `source` and `target` are the same pane
    /// - either is missing from the tree
    /// - `source` is the root (cannot remove the only leaf)
    pub fn reparent(&mut self, source: PaneId, target: PaneId, edge: Edge) -> bool {
        if source == target {
            return false;
        }
        let leaves = self.leaves();
        if !leaves.contains(&source) || !leaves.contains(&target) {
            return false;
        }
        // Snapshot for rollback if anything fails mid-way.
        let backup = self.clone();
        if !self.remove_leaf(source) {
            return false;
        }
        let placed = self.replace_leaf(target, |target_id| {
            let (dir, a, b) = match edge {
                Edge::Top => (Dir::Horizontal, source, target_id),
                Edge::Bottom => (Dir::Horizontal, target_id, source),
                Edge::Left => (Dir::Vertical, source, target_id),
                Edge::Right => (Dir::Vertical, target_id, source),
            };
            Layout::Split {
                dir,
                a: Box::new(Layout::Leaf(a)),
                b: Box::new(Layout::Leaf(b)),
                ratio: 0.5,
            }
        });
        if !placed {
            *self = backup;
            return false;
        }
        true
    }
}

fn replace_leaf_with<F>(node: &mut Layout, target: PaneId, f: &mut Option<F>) -> bool
where
    F: FnOnce(PaneId) -> Layout,
{
    match node {
        Layout::Leaf(id) if *id == target => {
            let Some(func) = f.take() else {
                return false;
            };
            *node = func(target);
            true
        }
        Layout::Split { a, b, .. } => {
            if replace_leaf_with(a, target, f) {
                return true;
            }
            replace_leaf_with(b, target, f)
        }
        _ => false,
    }
}

fn remove_leaf_in(node: &mut Layout, target: PaneId) -> bool {
    let Layout::Split { a, b, .. } = node else {
        return false;
    };
    // Case: target is a direct leaf child — promote the sibling.
    if let Layout::Leaf(id) = **a {
        if id == target {
            let sibling = std::mem::replace(b.as_mut(), Layout::Leaf(0));
            *node = sibling;
            return true;
        }
    }
    if let Layout::Leaf(id) = **b {
        if id == target {
            let sibling = std::mem::replace(a.as_mut(), Layout::Leaf(0));
            *node = sibling;
            return true;
        }
    }
    // Recurse into splits.
    if remove_leaf_in(a, target) {
        return true;
    }
    remove_leaf_in(b, target)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn leaves_in_order() {
        let tree = Layout::Split {
            dir: Dir::Horizontal,
            a: Box::new(Layout::Split {
                dir: Dir::Vertical,
                a: Box::new(Layout::Leaf(1)),
                b: Box::new(Layout::Leaf(2)),
                ratio: 0.5,
            }),
            b: Box::new(Layout::Leaf(3)),
            ratio: 0.5,
        };
        assert_eq!(tree.leaves(), vec![1, 2, 3]);
    }

    #[test]
    fn remove_promotes_sibling() {
        let mut tree = Layout::Split {
            dir: Dir::Horizontal,
            a: Box::new(Layout::Leaf(1)),
            b: Box::new(Layout::Leaf(2)),
            ratio: 0.5,
        };
        assert!(tree.remove_leaf(2));
        assert_eq!(tree.leaves(), vec![1]);
    }

    #[test]
    fn cannot_remove_last_leaf() {
        let mut tree = Layout::Leaf(1);
        assert!(!tree.remove_leaf(1));
        assert_eq!(tree.leaves(), vec![1]);
    }

    #[test]
    fn edge_from_xy_picks_closest() {
        // Cursor in the upper band → Top.
        assert_eq!(Edge::from_xy(50.0, 5.0, 100.0, 100.0), Edge::Top);
        // Lower band → Bottom.
        assert_eq!(Edge::from_xy(50.0, 95.0, 100.0, 100.0), Edge::Bottom);
        // Left band → Left.
        assert_eq!(Edge::from_xy(5.0, 50.0, 100.0, 100.0), Edge::Left);
        // Right band → Right.
        assert_eq!(Edge::from_xy(95.0, 50.0, 100.0, 100.0), Edge::Right);
    }

    #[test]
    fn reparent_into_right_splits_target_vertically() {
        let mut tree = Layout::Split {
            dir: Dir::Horizontal,
            a: Box::new(Layout::Leaf(1)),
            b: Box::new(Layout::Leaf(2)),
            ratio: 0.5,
        };
        // Drop pane 1 onto pane 2's right edge → 2 sits left, 1 right.
        assert!(tree.reparent(1, 2, Edge::Right));
        assert_eq!(tree.leaves(), vec![2, 1]);
        match tree {
            Layout::Split {
                dir: Dir::Vertical, ..
            } => {}
            other => panic!("expected vertical split, got {other:?}"),
        }
    }

    #[test]
    fn reparent_into_top_splits_target_horizontally() {
        let mut tree = Layout::Split {
            dir: Dir::Vertical,
            a: Box::new(Layout::Leaf(1)),
            b: Box::new(Layout::Leaf(2)),
            ratio: 0.5,
        };
        // Drop pane 2 onto pane 1's top edge → 2 on top, 1 below.
        assert!(tree.reparent(2, 1, Edge::Top));
        assert_eq!(tree.leaves(), vec![2, 1]);
        match tree {
            Layout::Split {
                dir: Dir::Horizontal,
                ..
            } => {}
            other => panic!("expected horizontal split, got {other:?}"),
        }
    }

    #[test]
    fn reparent_rejects_self_drop_and_missing_panes() {
        let mut tree = Layout::Split {
            dir: Dir::Horizontal,
            a: Box::new(Layout::Leaf(1)),
            b: Box::new(Layout::Leaf(2)),
            ratio: 0.5,
        };
        let snapshot = format!("{tree:?}");
        // Self-drop is a no-op.
        assert!(!tree.reparent(1, 1, Edge::Right));
        // Missing source.
        assert!(!tree.reparent(99, 2, Edge::Right));
        // Missing target.
        assert!(!tree.reparent(1, 99, Edge::Right));
        assert_eq!(format!("{tree:?}"), snapshot);
    }

    #[test]
    fn split_replaces_leaf() {
        let mut tree = Layout::Leaf(1);
        assert!(tree.replace_leaf(1, |id| Layout::Split {
            dir: Dir::Horizontal,
            a: Box::new(Layout::Leaf(id)),
            b: Box::new(Layout::Leaf(2)),
            ratio: 0.5,
        }));
        assert_eq!(tree.leaves(), vec![1, 2]);
    }
}

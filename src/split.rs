use std::path::PathBuf;
use std::time::Instant;

use crate::config::Config;
use crate::pty::PtySession;
use crate::render::TerminalLayout;
use crate::render::{split_tree_rects, ContentRect, SplitLayoutEntry};
use crate::tabs::{EventLoopProxyFactory, TabId};

pub const MIN_SPLIT_RATIO: f32 = 0.15;
pub const MAX_LEAVES: usize = 9;
pub const SPLIT_DIVIDER: f32 = 4.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SplitDirection {
    /// Side-by-side panes (vertical divider).
    Horizontal,
    /// Stacked panes (horizontal divider).
    Vertical,
}

pub enum SplitNode {
    /// Transient placeholder during tree restructuring.
    Pending,
    Leaf {
        session: PtySession,
        running_since: Option<Instant>,
    },
    Split {
        direction: SplitDirection,
        ratio: f32,
        first: Box<SplitNode>,
        second: Box<SplitNode>,
    },
}

impl SplitNode {
    pub fn leaf(session: PtySession) -> Self {
        Self::Leaf {
            session,
            running_since: None,
        }
    }

    pub fn leaf_count(&self) -> usize {
        match self {
            Self::Pending => 0,
            Self::Leaf { .. } => 1,
            Self::Split { first, second, .. } => first.leaf_count() + second.leaf_count(),
        }
    }

    pub fn leaf_session(&self, leaf_id: usize) -> Option<&PtySession> {
        match self {
            Self::Leaf { session, .. } => (leaf_id == 0).then_some(session),
            Self::Split { first, second, .. } => {
                let first_count = first.leaf_count();
                if leaf_id < first_count {
                    first.leaf_session(leaf_id)
                } else {
                    second.leaf_session(leaf_id - first_count)
                }
            }
            Self::Pending => None,
        }
    }

    pub fn leaf_session_mut(&mut self, leaf_id: usize) -> Option<&mut PtySession> {
        match self {
            Self::Leaf { session, .. } => (leaf_id == 0).then_some(session),
            Self::Split { first, second, .. } => {
                let first_count = first.leaf_count();
                if leaf_id < first_count {
                    first.leaf_session_mut(leaf_id)
                } else {
                    second.leaf_session_mut(leaf_id - first_count)
                }
            }
            Self::Pending => None,
        }
    }

    pub fn leaf_running_since(&self, leaf_id: usize) -> Option<Option<Instant>> {
        match self {
            Self::Leaf { running_since, .. } => (leaf_id == 0).then_some(*running_since),
            Self::Split { first, second, .. } => {
                let first_count = first.leaf_count();
                if leaf_id < first_count {
                    first.leaf_running_since(leaf_id)
                } else {
                    second.leaf_running_since(leaf_id - first_count)
                }
            }
            Self::Pending => None,
        }
    }

    pub fn leaf_running_since_mut(&mut self, leaf_id: usize) -> Option<&mut Option<Instant>> {
        match self {
            Self::Leaf { running_since, .. } => (leaf_id == 0).then_some(running_since),
            Self::Split { first, second, .. } => {
                let first_count = first.leaf_count();
                if leaf_id < first_count {
                    first.leaf_running_since_mut(leaf_id)
                } else {
                    second.leaf_running_since_mut(leaf_id - first_count)
                }
            }
            Self::Pending => None,
        }
    }

    pub fn any_busy(&self, heuristic_ms: u64) -> bool {
        match self {
            Self::Leaf { session, .. } => session.is_busy(heuristic_ms),
            Self::Split { first, second, .. } => {
                first.any_busy(heuristic_ms) || second.any_busy(heuristic_ms)
            }
            Self::Pending => false,
        }
    }

    pub fn focused_cwd(&self, focused_leaf: usize) -> Option<PathBuf> {
        self.leaf_session(focused_leaf)?.current_working_directory()
    }

    pub fn layout_entries(
        &self,
        rect: ContentRect,
        leaf_start: &mut usize,
    ) -> Vec<SplitLayoutEntry> {
        split_tree_rects(rect, self, leaf_start)
    }

    pub fn split_leaf(
        &mut self,
        leaf_id: usize,
        direction: SplitDirection,
        new_session: PtySession,
    ) -> Option<usize> {
        if self.leaf_count() >= MAX_LEAVES {
            return None;
        }
        let new_id = self.leaf_count();
        split_leaf_inner(self, leaf_id, direction, new_session)?;
        Some(new_id)
    }

    pub fn remove_leaf(&mut self, leaf_id: usize) -> bool {
        if self.leaf_count() <= 1 {
            return false;
        }
        remove_leaf_inner(self, leaf_id).is_some()
    }

    pub fn adjust_ratio_at_divider(&mut self, divider_index: usize, delta_ratio: f32) {
        adjust_ratio(self, divider_index, delta_ratio);
    }

    pub fn divider_count(&self) -> usize {
        match self {
            Self::Leaf { .. } | Self::Pending => 0,
            Self::Split { first, second, .. } => 1 + first.divider_count() + second.divider_count(),
        }
    }
}

fn split_leaf_inner(
    node: &mut SplitNode,
    target: usize,
    direction: SplitDirection,
    new_session: PtySession,
) -> Option<()> {
    match node {
        SplitNode::Leaf { .. } if target == 0 => {
            let new = SplitNode::leaf(new_session);
            let _old = std::mem::replace(node, new);
            let new_leaf = std::mem::replace(node, _old);
            let first = std::mem::replace(node, SplitNode::Pending);
            *node = SplitNode::Split {
                direction,
                ratio: 0.5,
                first: Box::new(first),
                second: Box::new(new_leaf),
            };
            Some(())
        }
        SplitNode::Split { first, second, .. } => {
            let first_count = first.leaf_count();
            if target < first_count {
                split_leaf_inner(first, target, direction, new_session)
            } else {
                split_leaf_inner(second, target - first_count, direction, new_session)
            }
        }
        _ => None,
    }
}

fn remove_leaf_inner(node: &mut SplitNode, target: usize) -> Option<()> {
    match node {
        SplitNode::Leaf { .. } => None,
        SplitNode::Split {
            first,
            second,
            direction,
            ratio,
        } => {
            let first_count = first.leaf_count();
            if target < first_count {
                if first_count == 1 && target == 0 {
                    let sibling = std::mem::replace(second.as_mut(), SplitNode::Pending);
                    let _ = std::mem::replace(node, sibling);
                    return Some(());
                }
                if remove_leaf_inner(first, target).is_some() {
                    return Some(());
                }
            } else {
                let idx = target - first_count;
                if second.leaf_count() == 1 && idx == 0 {
                    let sibling = std::mem::replace(first.as_mut(), SplitNode::Pending);
                    let _ = std::mem::replace(node, sibling);
                    return Some(());
                }
                if remove_leaf_inner(second, idx).is_some() {
                    return Some(());
                }
            }
            let _ = (direction, ratio);
            None
        }
        SplitNode::Pending => None,
    }
}

pub fn spawn_leaf_session(
    config: &Config,
    tab_id: TabId,
    leaf_id: usize,
    layout: TerminalLayout,
    proxy: &EventLoopProxyFactory,
    working_directory: Option<PathBuf>,
) -> Result<PtySession, anyhow::Error> {
    let event_proxy = proxy.for_tab(tab_id, leaf_id);
    PtySession::spawn_with_working_directory(config, layout, event_proxy, working_directory)
}

fn adjust_ratio(node: &mut SplitNode, divider_index: usize, delta_ratio: f32) {
    match node {
        SplitNode::Leaf { .. } | SplitNode::Pending => {}
        SplitNode::Split {
            ratio,
            first,
            second,
            ..
        } => {
            if divider_index == 0 {
                *ratio = (*ratio + delta_ratio).clamp(MIN_SPLIT_RATIO, 1.0 - MIN_SPLIT_RATIO);
                return;
            }
            let first_dividers = first.divider_count();
            if divider_index <= first_dividers {
                adjust_ratio(first, divider_index - 1, delta_ratio);
            } else {
                adjust_ratio(second, divider_index - first_dividers - 1, delta_ratio);
            }
        }
    }
}

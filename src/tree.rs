//! Queries over the sway/i3 layout tree and outputs.

use swayipc::{Node, NodeType, Output};

use crate::geometry::Rect;

#[derive(Debug, PartialEq, Eq)]
pub struct Marked {
    pub id: i64,
    /// Name of the enclosing workspace (`__i3_scratch` when hidden).
    pub workspace: Option<String>,
    /// Outer geometry, title bar included, in absolute coordinates.
    pub rect: Rect,
}

/// Depth-first search over tiled and floating children. Returns the first
/// node matching `pred`, with the name of its enclosing workspace.
fn find<'a>(root: &'a Node, pred: &impl Fn(&Node) -> bool) -> Option<(&'a Node, Option<&'a str>)> {
    fn walk<'a>(
        node: &'a Node,
        workspace: Option<&'a str>,
        pred: &impl Fn(&Node) -> bool,
    ) -> Option<(&'a Node, Option<&'a str>)> {
        let workspace = match node.node_type {
            NodeType::Workspace => node.name.as_deref(),
            _ => workspace,
        };
        if pred(node) {
            return Some((node, workspace));
        }
        node.nodes
            .iter()
            .chain(&node.floating_nodes)
            .find_map(|child| walk(child, workspace, pred))
    }
    walk(root, None, pred)
}

/// Finds the container carrying `mark`, along with its workspace.
pub fn find_marked(root: &Node, mark: &str) -> Option<Marked> {
    let (node, workspace) = find(root, &|n| n.marks.iter().any(|m| m == mark))?;
    Some(Marked {
        id: node.id,
        workspace: workspace.map(str::to_owned),
        rect: Rect {
            x: node.rect.x,
            y: node.rect.y - node.deco_rect.height,
            w: node.rect.width,
            h: node.rect.height + node.deco_rect.height,
        },
    })
}

/// The workspace holding the focused container (or the focused, empty
/// workspace itself). Read from the same tree snapshot as `find_marked`, so
/// a focus change in between cannot flip the toggle decision.
pub fn focused_workspace(root: &Node) -> Option<&str> {
    find(root, &|n| n.focused)?.1
}

pub fn contains(root: &Node, id: i64) -> bool {
    find(root, &|n| n.id == id).is_some()
}

/// The requested output if it exists, else the focused one (sway), the
/// primary one (i3), or the first active one.
pub fn pick_output<'a>(outputs: &'a [Output], name: Option<&str>) -> Option<&'a Output> {
    let active = || outputs.iter().filter(|o| o.active);
    name.and_then(|n| active().find(|o| o.name == n))
        .or_else(|| active().find(|o| o.focused))
        .or_else(|| active().find(|o| o.primary))
        .or_else(|| active().next())
}

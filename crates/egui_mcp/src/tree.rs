//! Helpers that flatten the accesskit tree into MCP-friendly shapes.
//!
//! Note: `accesskit_consumer::NodeId` is a private composite (tree-index + local-id) and
//! can't be constructed from outside the crate. We project everything externally as the
//! original `accesskit::NodeId` (a `pub u64`), and look up by walking the tree.

use accesskit_consumer::{NodeRef, Tree};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// `accesskit::Role::Unknown`, as `{:?}` spells it. What egui reports for a widget that
/// declares no kind (`WidgetType::Other`), so it tells an agent nothing.
const UNKNOWN_ROLE: &str = "Unknown";

/// `accesskit::Role::GenericContainer`, as `{:?}` spells it. egui's layout scaffolding: a
/// `Ui`, a row, a panel's frame. It is a place, not a widget.
const GENERIC_CONTAINER_ROLE: &str = "GenericContainer";

/// `accesskit::Role::TextRun`, as `{:?}` spells it. egui puts one under every text widget, one
/// per laid-out line, repeating text the widget above already carries.
const TEXT_RUN_ROLE: &str = "TextRun";

/// How much of a widget's `label`/`value` [`Widget`] carries before cutting it short.
///
/// A widget's text is unbounded — a text editor's contents, a log line, a chat message — and
/// egui exports even the scrolled-out parts of a `ScrollArea`, so a whole-tree query can hand
/// back far more text than an agent needs to find its way around. Filters still match the full
/// text, and `get_widget` still returns it.
pub const MAX_TEXT_CHARS: usize = 100;

/// Marks text cut short at [`MAX_TEXT_CHARS`]. Distinct enough that an agent can tell it from
/// an ellipsis the app itself drew.
pub const TRUNCATION_MARKER: &str = " […]";

/// A widget id: the original `accesskit::NodeId`, written as lower-case hex with no prefix.
///
/// Hex keeps ids short, which matters because a `widget_tree` result is mostly ids. The type
/// exists so a malformed id is rejected where it is read, rather than silently becoming "no
/// id" — a mistyped `exclude.id` used to leave the subtree in the result, and say nothing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Id(pub u64);

impl std::fmt::Display for Id {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:x}", self.0)
    }
}

impl std::str::FromStr for Id {
    type Err = String;

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        u64::from_str_radix(text.trim(), 16).map(Self).map_err(|err| {
            format!("invalid widget id `{text}` ({err}) — pass an `id` exactly as `widget_tree` returned it")
        })
    }
}

impl Serialize for Id {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_str(self)
    }
}

impl<'de> Deserialize<'de> for Id {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let text = String::deserialize(deserializer)?;
        text.parse().map_err(serde::de::Error::custom)
    }
}

impl JsonSchema for Id {
    fn schema_name() -> std::borrow::Cow<'static, str> {
        "Id".into()
    }

    fn json_schema(_generator: &mut schemars::SchemaGenerator) -> schemars::Schema {
        schemars::json_schema!({
            "type": "string",
            "pattern": "^[0-9a-fA-F]{1,16}$",
            "description": "Widget id as `widget_tree` reports it: lower-case hex, no prefix.",
        })
    }
}

#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct WidgetDetail {
    /// Node id, used with `click`, `type_text`, and `get_widget`.
    pub id: Id,
    pub role: String,
    pub label: Option<String>,
    pub value: Option<String>,
    pub bounds: Option<RectF>,
    pub focused: bool,
    pub disabled: bool,
    pub hidden: bool,
    pub parent_id: Option<Id>,
}

/// A widget in `widget_tree`'s hierarchical result.
///
/// Only matching widgets appear; each one nests the matches found in its own subtree. A
/// non-matching widget contributes its matches to its nearest matching ancestor, so a
/// `children` entry is not necessarily a direct child in the app's tree.
///
/// Deliberately leaner than [`WidgetDetail`]: a whole tree of `bounds` is a lot of numbers for an
/// agent to read past, and actions take an `id` anyway. `get_widget` has the full detail.
///
/// A widget that says nothing — no children, no `label`, no `value` and no role — is dropped
/// entirely, as is a `TextRun` that only repeats its parent's text.
///
/// `label` and `value` are cut short at [`MAX_TEXT_CHARS`], marked with a trailing
/// [`TRUNCATION_MARKER`].
#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct Widget {
    /// Node id, used with `click`, `type_text`, and `get_widget`.
    pub id: Id,

    /// Omitted for a role of `Unknown`, which carries no information.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub role: Option<String>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub value: Option<String>,

    /// Each flag is omitted when false — that is the state of nearly every widget.
    #[serde(default, skip_serializing_if = "is_false")]
    pub focused: bool,

    #[serde(default, skip_serializing_if = "is_false")]
    pub disabled: bool,

    #[serde(default, skip_serializing_if = "is_false")]
    pub hidden: bool,

    /// `default` keeps the derived schema honest: a leaf omits the field entirely.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub children: Vec<Self>,

    /// How many of this widget's children `limit` left out. Query again with this widget's
    /// `id` as `root` to see them, or raise `limit`, or narrow the filter.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub omitted_children: usize,
}

impl Widget {
    /// Does this widget only stand between its parent and its one child?
    ///
    /// egui nests layout containers several deep — a panel inside a frame inside a `Ui` — and a
    /// chain of them with one child each is a chain of nothing: no text, no state, nothing to
    /// click. The child says everything the chain does.
    fn is_pass_through(&self, filter: &QueryFilter) -> bool {
        self.is_silent_container(filter) && self.children.len() == 1
    }

    /// Is this a layout container with nothing of its own to say — no text, no state?
    ///
    /// Such a container only adds a level of nesting. It collapses when it holds one child
    /// ([`Self::is_pass_through`]), or when it is the one child of its parent, which then
    /// adopts its children.
    ///
    /// Not collapsed when the caller asked for this role by name, since they would then get
    /// nothing back at all.
    fn is_silent_container(&self, filter: &QueryFilter) -> bool {
        let is_container = match self.role.as_deref() {
            None => true,
            Some(role) => role == GENERIC_CONTAINER_ROLE,
        };
        let asked_for =
            filter.query.role.as_deref().is_some_and(|wanted| {
                wanted.eq_ignore_ascii_case(self.role.as_deref().unwrap_or(""))
            });
        is_container
            && !asked_for
            && self.label.is_none()
            && self.value.is_none()
            && !self.focused
            && !self.disabled
            && !self.hidden
    }

    /// A childless widget with no `label`, no `value` and no role says nothing an agent can act
    /// on or read — it is layout scaffolding that survived the filter. `widget_tree` drops it.
    fn is_noise(&self) -> bool {
        self.children.is_empty()
            && self.label.is_none()
            && self.value.is_none()
            && self.role.is_none()
    }
}

/// For `#[serde(skip_serializing_if)]`.
#[expect(clippy::trivially_copy_pass_by_ref)]
fn is_false(b: &bool) -> bool {
    !*b
}

/// For `#[serde(skip_serializing_if)]`.
#[expect(clippy::trivially_copy_pass_by_ref)]
fn is_zero(n: &usize) -> bool {
    *n == 0
}

#[derive(Debug, Clone, Copy, Serialize, JsonSchema)]
pub struct RectF {
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
}

impl RectF {
    /// Build from `AccessKit` physical-pixel bounds, scaling to logical points (the consumer's
    /// `bounding_box` applies egui's root `scale(pixels_per_point)` transform, so bounds arrive
    /// in physical pixels — divide them back out).
    fn from_physical(r: accesskit::Rect, pixels_per_point: f32) -> Self {
        let s = 1.0 / f64::from(pixels_per_point);
        Self {
            x: r.x0 * s,
            y: r.y0 * s,
            w: (r.x1 - r.x0) * s,
            h: (r.y1 - r.y0) * s,
        }
    }

    pub fn center(&self) -> (f64, f64) {
        (self.x + self.w / 2.0, self.y + self.h / 2.0)
    }
}

/// The widget-matching constraints shared by `widget_tree`'s filter and the action `Target`s.
///
/// An optional `role` plus up to one text predicate. All are case-insensitive and combined with
/// logical AND; an all-`None` `Query` matches every widget.
#[derive(Debug, Clone, Default, Deserialize, JsonSchema)]
pub struct Query {
    /// Case-insensitive substring match against *either* `label` or `value`. Prefer this when you
    /// just want "the widget showing this text" and don't care which field holds it — it's the
    /// most robust choice across widget kinds.
    pub content_contains: Option<String>,
    /// Role name, e.g. `Button`, `Label`, `TextInput` (case-insensitive).
    /// An unrecognized role is rejected with an error that lists the roles present in the tree.
    pub role: Option<String>,
    /// Case-insensitive substring match against the widget's `label` (its accessible name) only.
    /// Note that `Label`/monospace widgets carry their text in `value`, not `label` — for those,
    /// use `content_contains` (or `value_contains`).
    pub label_contains: Option<String>,
    /// Case-insensitive substring match against the widget's `value` only (e.g. a text field's
    /// contents, or a `Label`'s text).
    pub value_contains: Option<String>,
}

impl Query {
    /// True when no constraint is set (matches every widget).
    pub fn is_empty(&self) -> bool {
        self.content_contains.is_none()
            && self.role.is_none()
            && self.label_contains.is_none()
            && self.value_contains.is_none()
    }

    /// Does `node` satisfy every set constraint? (Visibility is the caller's concern.)
    fn matches(&self, node: &NodeRef<'_>) -> bool {
        if let Some(needle) = &self.content_contains
            && !contains_ci(&node.label().unwrap_or_default(), needle)
            && !contains_ci(&node.value().unwrap_or_default(), needle)
        {
            return false;
        }
        if let Some(role) = &self.role
            && !role.eq_ignore_ascii_case(&format!("{:?}", node.role()))
        {
            return false;
        }
        if let Some(needle) = &self.label_contains
            && !contains_ci(&node.label().unwrap_or_default(), needle)
        {
            return false;
        }
        if let Some(needle) = &self.value_contains
            && !contains_ci(&node.value().unwrap_or_default(), needle)
        {
            return false;
        }
        true
    }

    /// Human-readable description of the constraints, for "matched N nodes" / "no node" errors.
    fn describe(&self) -> String {
        let mut clauses = Vec::new();
        if let Some(c) = &self.content_contains {
            clauses.push(format!("label or value containing `{c}`"));
        }
        if let Some(r) = &self.role {
            clauses.push(format!("role `{r}`"));
        }
        if let Some(l) = &self.label_contains {
            clauses.push(format!("label containing `{l}`"));
        }
        if let Some(v) = &self.value_contains {
            clauses.push(format!("value containing `{v}`"));
        }
        if clauses.is_empty() {
            "the locator".to_owned()
        } else {
            clauses.join(" and ")
        }
    }
}

/// A subtree to leave out of a query: whatever matches, plus everything below it.
///
/// Either a widget `id` or the same constraints a [`Query`] takes. An exclusion with neither set
/// matches nothing, rather than swallowing the whole tree.
#[derive(Debug, Clone, Default, Deserialize, JsonSchema)]
pub struct Exclusion {
    /// Id of the widget to leave out, from a previous query.
    #[serde(default)]
    pub id: Option<Id>,

    #[serde(flatten)]
    pub query: Query,
}

impl Exclusion {
    /// Does `node` root a subtree the caller asked to leave out?
    fn matches(&self, node: &NodeRef<'_>) -> bool {
        if let Some(id) = self.id
            && accesskit_id(node) == id
        {
            return true;
        }
        !self.query.is_empty() && self.query.matches(node)
    }
}

/// What subset of the tree `widget_tree` walks, and how much of it comes back.
///
/// A default filter — no constraints, no `root`, no `exclude` — returns the whole visible app,
/// up to `limit`. Every field narrows the result further; they combine with logical AND.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct QueryFilter {
    /// Which widgets count as matches. An empty [`Query`] matches every widget, so the result
    /// is the app's whole hierarchy rather than a search hit.
    #[serde(flatten)]
    pub query: Query,

    /// Walk this widget's subtree instead of the whole app — the way to see the children a
    /// `limit` left out, by passing the id of the widget that reported `omitted_children`.
    #[serde(default)]
    pub root: Option<Id>,

    /// A subtree to leave out — e.g. the host's own agent panel, whose text would otherwise
    /// answer every text query before the app under test does.
    #[serde(default)]
    pub exclude: Option<Exclusion>,

    /// Skip the widgets egui reports as hidden — scrolled out of view, or inside a collapsed
    /// section. On by default: a hidden widget can't be clicked or read by a user, so it is
    /// rarely the one an agent is looking for. Set it to `false` to see what a `ScrollArea` or
    /// a closed `CollapsingHeader` is holding.
    #[serde(default = "default_true")]
    pub visible_only: bool,

    /// Most widgets to return, counted over the whole forest and not per level. The default of
    /// 200 keeps a whole-app query down to something an agent can read.
    ///
    /// Overflow is cut from the bottom up, so the top of the hierarchy always survives, and
    /// each widget that lost children reports how many in its `omitted_children`.
    #[serde(default = "default_limit")]
    pub limit: usize,
}

fn default_true() -> bool {
    true
}

fn default_limit() -> usize {
    200
}

impl Default for QueryFilter {
    fn default() -> Self {
        Self {
            query: Query::default(),
            root: None,
            exclude: None,
            visible_only: true,
            limit: default_limit(),
        }
    }
}

/// Walk `tree`, returning the widgets that match `filter`, nested by ancestry.
///
/// # Errors
/// If `filter.root` names a widget the tree doesn't have.
pub fn query(
    tree: &Tree,
    filter: &QueryFilter,
    pixels_per_point: f32,
) -> Result<Vec<Widget>, String> {
    let root = match &filter.root {
        Some(id) => resolve_unique(tree, &Locator::Id { id: *id }, pixels_per_point)?,
        None => tree.state().root(),
    };
    let mut nodes = walk(&root, filter, pixels_per_point);
    truncate(&mut nodes, filter.limit);
    Ok(nodes)
}

/// The matches in `node`'s subtree (including `node` itself), nested by ancestry.
///
/// A non-matching node returns its descendants' matches, which its own parent then adopts. An
/// excluded node returns nothing at all — the exclusion takes its subtree with it.
fn walk(node: &NodeRef<'_>, filter: &QueryFilter, pixels_per_point: f32) -> Vec<Widget> {
    if filter
        .exclude
        .as_ref()
        .is_some_and(|exclude| exclude.matches(node))
    {
        return Vec::new();
    }
    let children: Vec<Widget> = node
        .children()
        .flat_map(|child| walk(&child, filter, pixels_per_point))
        .collect();
    if matches(node, filter) {
        let children = drop_echoed_text_runs(node, children);
        let mut view = to_widget(node, children, pixels_per_point);
        // An only child that is a silent container stands between this node and its own
        // children, so this node adopts them. That child already collapsed if it held just one,
        // so what it hands over is never another only child to collapse.
        if let [only_child] = view.children.as_mut_slice()
            && only_child.is_silent_container(filter)
        {
            view.children = std::mem::take(&mut only_child.children);
        }
        // Pruning runs bottom-up, so a node left childless by it is reconsidered here in turn,
        // and a chain of containers collapses a link at a time.
        if view.is_noise() {
            Vec::new()
        } else if view.is_pass_through(filter) {
            std::mem::take(&mut view.children)
        } else {
            vec![view]
        }
    } else {
        children
    }
}

/// Cut `text` short at [`MAX_TEXT_CHARS`], marking the cut with a trailing
/// [`TRUNCATION_MARKER`].
///
/// Counted in characters, not bytes, so the cut lands on a character boundary.
fn shorten(text: String) -> String {
    if text.chars().count() <= MAX_TEXT_CHARS {
        return text;
    }
    let kept: String = text.chars().take(MAX_TEXT_CHARS).collect();
    format!("{kept}{TRUNCATION_MARKER}")
}

/// Drop the `TextRun` children whose text `parent` already carries.
///
/// egui lays a string out as one `TextRun` per line under the widget that owns it, so the text
/// comes back twice — and a wrapped paragraph comes back once per line on top of that. A run is
/// not a widget an agent can act on: actions target the parent, which still holds the whole
/// string. A run whose text the parent *doesn't* carry is left alone, since dropping it would
/// lose the only copy.
fn drop_echoed_text_runs(parent: &NodeRef<'_>, children: Vec<Widget>) -> Vec<Widget> {
    let owned_text = format!(
        "{} {}",
        parent.label().unwrap_or_default(),
        parent.value().unwrap_or_default()
    );
    children
        .into_iter()
        .filter(|child| {
            let is_echo = child.role.as_deref() == Some(TEXT_RUN_ROLE)
                && child.children.is_empty()
                && child.value.as_deref().is_some_and(|text| {
                    // The run's own text may have been cut short, so compare the part of it
                    // that survived.
                    owned_text.contains(text.trim_end_matches(TRUNCATION_MARKER))
                });
            !is_echo
        })
        .collect()
}

/// Cut the forest down to `budget` nodes, from the bottom up.
///
/// Every level that fits whole is kept whole, so an agent always gets the top of the app's
/// hierarchy — the panels and sections it navigates by — and loses the leaves first. What is
/// left of the budget after the last full level is spent on the level below it, in order.
///
/// A node whose children were cut records how many in `omitted_children`, so the agent can see
/// that there is more and ask for it by that node's `id`.
fn truncate(nodes: &mut Vec<Widget>, budget: usize) {
    let full_depth = (0..height(nodes)).rfind(|&depth| count_within(nodes, depth) <= budget);

    let Some(full_depth) = full_depth else {
        // Not even the roots fit. There is no parent to record the loss on, so this is the one
        // cut an agent can't see; `limit` would have to be pathologically small.
        nodes.truncate(budget);
        for node in nodes.iter_mut() {
            node.omitted_children = node.children.len();
            node.children.clear();
        }
        return;
    };

    let mut extra = budget - count_within(nodes, full_depth);
    prune(nodes, full_depth, &mut extra);
}

/// Depth of the deepest node in the forest, counting a forest of leaves as 1.
fn height(nodes: &[Widget]) -> usize {
    nodes
        .iter()
        .map(|node| 1 + height(&node.children))
        .max()
        .unwrap_or(0)
}

/// How many nodes there are down to and including `depth` (`0` is the roots).
fn count_within(nodes: &[Widget], depth: usize) -> usize {
    nodes
        .iter()
        .map(|node| {
            if depth == 0 {
                1
            } else {
                1 + count_within(&node.children, depth - 1)
            }
        })
        .sum()
}

/// Keep every node down to `depth_left`, then spend `extra` on the level below it.
fn prune(nodes: &mut [Widget], depth_left: usize, extra: &mut usize) {
    for node in nodes {
        if 0 < depth_left {
            prune(&mut node.children, depth_left - 1, extra);
            continue;
        }
        let kept = node.children.len().min(*extra);
        *extra -= kept;
        node.omitted_children = node.children.len() - kept;
        node.children.truncate(kept);
        for child in &mut node.children {
            child.omitted_children = child.children.len();
            child.children.clear();
        }
    }
}

/// Total number of nodes in a forest of [`Widget`]s.
pub fn count(nodes: &[Widget]) -> usize {
    nodes.iter().map(|node| 1 + count(&node.children)).sum()
}

/// Validate a `role` filter string against the full `AccessKit` role set.
///
/// Compared case-insensitively, the way `matches` compares. On failure the error lists the
/// distinct roles actually present in `tree`, so the agent learns what it can filter by instead of
/// getting a silent empty result. Validity is checked against *all* roles, not just those present,
/// so polling tools like `wait_for` can still wait for a valid role that hasn't appeared yet.
///
/// # Errors
/// If `role` is not a known `AccessKit` role name.
pub fn validate_role(role: &str, tree: Option<&Tree>) -> Result<(), String> {
    // `accesskit::Role` is `#[repr(u8)]` with `enumn::N`, so walking `n(0), n(1), …` until `None`
    // enumerates every variant; `{:?}` yields the same name `matches`/`widget_detail` expose.
    let valid = (0u8..=u8::MAX)
        .map_while(accesskit::Role::n)
        .any(|r| role.eq_ignore_ascii_case(&format!("{r:?}")));
    if valid {
        return Ok(());
    }
    let present = tree.map(roles_in_tree).unwrap_or_default();
    let hint = if present.is_empty() {
        "(no nodes in the current tree)".to_owned()
    } else {
        present.join(", ")
    };
    Err(format!(
        "unknown role `{role}` — roles present in the current tree: {hint}"
    ))
}

/// The distinct `AccessKit` roles present anywhere in `tree`, sorted, as their display names.
fn roles_in_tree(tree: &Tree) -> Vec<String> {
    let mut roles = std::collections::BTreeSet::new();
    collect_roles(&tree.state().root(), &mut roles);
    roles.into_iter().collect()
}

fn collect_roles(node: &NodeRef<'_>, out: &mut std::collections::BTreeSet<String>) {
    out.insert(format!("{:?}", node.role()));
    for child in node.children() {
        collect_roles(&child, out);
    }
}

fn matches(node: &NodeRef<'_>, filter: &QueryFilter) -> bool {
    if filter.visible_only && node.is_hidden() {
        return false;
    }
    filter.query.matches(node)
}

/// Case-insensitive substring test (ASCII-folded, matching how `role` is compared).
fn contains_ci(hay: &str, needle: &str) -> bool {
    hay.to_ascii_lowercase()
        .contains(&needle.to_ascii_lowercase())
}

pub fn widget_detail(node: &NodeRef<'_>, pixels_per_point: f32) -> WidgetDetail {
    WidgetDetail {
        id: accesskit_id(node),
        role: format!("{:?}", node.role()),
        label: node.label(),
        value: node.value(),
        bounds: node
            .bounding_box()
            .map(|r| RectF::from_physical(r, pixels_per_point)),
        focused: node.is_focused_in_tree(),
        disabled: node.is_disabled(),
        hidden: node.is_hidden(),
        parent_id: node.parent().map(|p| accesskit_id(&p)),
    }
}

fn to_widget(node: &NodeRef<'_>, children: Vec<Widget>, pixels_per_point: f32) -> Widget {
    let WidgetDetail {
        id,
        role,
        label,
        value,
        bounds: _,
        focused,
        disabled,
        hidden,
        parent_id: _,
    } = widget_detail(node, pixels_per_point);
    Widget {
        id,
        role: (role != UNKNOWN_ROLE).then_some(role),
        label: label.map(shorten),
        value: value.map(shorten),
        focused,
        disabled,
        hidden,
        children,
        omitted_children: 0,
    }
}

/// Project a consumer node to its original `accesskit::NodeId`.
pub fn accesskit_id(node: &NodeRef<'_>) -> Id {
    let (local, _tree) = node.locate();
    Id(local.0)
}

/// A resolved lookup target: a specific node `id`, or a role/text match.
/// Built directly by the tools from a `Target` — never deserialized.
#[derive(Debug, Clone)]
pub enum Locator {
    Id { id: Id },
    Match { query: Query },
}

impl Locator {
    /// Build a locator from raw tool fields: an `id` wins, else the `query` constraints.
    /// Returns `None` when neither an `id` nor any `query` constraint is set.
    pub fn from_fields(id: Option<Id>, query: Query) -> Option<Self> {
        if let Some(id) = id {
            return Some(Self::Id { id });
        }
        if !query.is_empty() {
            return Some(Self::Match { query });
        }
        None
    }
}

/// Resolve a locator to *exactly one* node for an action (click, focus, …).
///
/// Like kittest's `get_by_*`, this is strict: an ambiguous locator is an error, not a silent
/// "first match wins". A specific `id` resolves at most one node; a `role`/text match
/// errors if it hits zero or more than one node, listing the candidates so the caller can narrow
/// the filter or target a specific `id`. Use `widget_tree` (which returns all matches) when you
/// genuinely expect several.
///
/// # Errors
/// If no node matches, or if more than one does.
pub fn resolve_unique<'a>(
    tree: &'a Tree,
    locator: &Locator,
    pixels_per_point: f32,
) -> Result<NodeRef<'a>, String> {
    let root = tree.state().root();
    match locator {
        Locator::Id { id } => {
            let mut found = Vec::new();
            find_all(&root, &|n| accesskit_id(n) == *id, &mut found);
            one(found, pixels_per_point, &format!("id `{id}`"))
        }
        Locator::Match { query } => {
            let filter = QueryFilter {
                query: query.clone(),
                visible_only: true,
                limit: usize::MAX,
                ..Default::default()
            };
            let mut found = Vec::new();
            find_all(&root, &|n| matches(n, &filter), &mut found);
            one(found, pixels_per_point, &query.describe())
        }
    }
}

/// Reduce a match list to the single node an action needs, or an error describing the miss.
fn one<'a>(
    mut found: Vec<NodeRef<'a>>,
    pixels_per_point: f32,
    what: &str,
) -> Result<NodeRef<'a>, String> {
    match found.len() {
        0 => Err(format!("no node found matching {what}")),
        1 => Ok(found.remove(0)),
        n => {
            let views: Vec<WidgetDetail> = found
                .iter()
                .map(|node| widget_detail(node, pixels_per_point))
                .collect();
            let list =
                serde_json::to_string_pretty(&views).unwrap_or_else(|_| format!("{n} nodes"));
            Err(format!(
                "{what} matched {n} nodes — narrow the locator (`content_contains`/`role`/`label_contains`/`value_contains`), or target a specific `id`. Matches:\n{list}"
            ))
        }
    }
}

/// Depth-first collection of every node satisfying `pred`.
fn find_all<'a>(
    node: &NodeRef<'a>,
    pred: &impl Fn(&NodeRef<'_>) -> bool,
    out: &mut Vec<NodeRef<'a>>,
) {
    if pred(node) {
        out.push(*node);
    }
    for child in node.children() {
        find_all(&child, pred, out);
    }
}

#[cfg(test)]
mod tests {
    use accesskit::{Node as AkNode, NodeId, Role, TreeId, TreeInfo, TreeUpdate};

    use super::*;

    /// ```text
    /// root(Window)
    /// ├── scaffold(Unknown)          — holds two things, so it stays
    /// │   ├── button(Button "OK")
    /// │   └── toggle(CheckBox "On")
    /// ├── wrapper(GenericContainer)  — holds one thing and says nothing, so it collapses
    /// │   └── link(Link "Docs")
    /// ├── text(Unknown, label "hi")
    /// ├── valued(Unknown, value "42")
    /// └── noise(Unknown)             — says nothing at all, so it goes
    /// ```
    fn test_tree() -> Tree {
        let mut root = AkNode::new(Role::Window);
        root.set_children(vec![
            NodeId(0x2),
            NodeId(0x7),
            NodeId(0xff),
            NodeId(0x5),
            NodeId(0x4),
        ]);
        let mut scaffold = AkNode::new(Role::Unknown);
        scaffold.set_children(vec![NodeId(0x3), NodeId(0x6)]);
        let mut button = AkNode::new(Role::Button);
        button.set_label("OK");
        let mut toggle = AkNode::new(Role::CheckBox);
        toggle.set_label("On");
        let mut wrapper = AkNode::new(Role::GenericContainer);
        wrapper.set_children(vec![NodeId(0x8)]);
        let mut link = AkNode::new(Role::Link);
        link.set_label("Docs");
        let mut text = AkNode::new(Role::Unknown);
        text.set_label("hi");
        let mut valued = AkNode::new(Role::Unknown);
        valued.set_value("42");
        let noise = AkNode::new(Role::Unknown);

        Tree::new(
            TreeUpdate {
                nodes: vec![
                    (NodeId(0x1), root),
                    (NodeId(0x2), scaffold),
                    (NodeId(0x3), button),
                    (NodeId(0x6), toggle),
                    (NodeId(0x7), wrapper),
                    (NodeId(0x8), link),
                    (NodeId(0xff), text),
                    (NodeId(0x5), valued),
                    (NodeId(0x4), noise),
                ],
                tree: Some(TreeInfo::new(NodeId(0x1))),
                tree_id: TreeId::ROOT,
                focus: NodeId(0x1),
            },
            false,
        )
    }

    fn query_all(filter: &QueryFilter) -> Vec<Widget> {
        query(&test_tree(), filter, 1.0).expect("the filter names no missing `root`")
    }

    #[test]
    fn unfiltered_query_keeps_the_hierarchy_and_drops_scaffolding() {
        let nodes = query_all(&QueryFilter::default());
        assert_eq!(nodes.len(), 1, "one root");
        let root = &nodes[0];
        assert_eq!(root.id, Id(0x1));
        assert_eq!(root.role.as_deref(), Some("Window"));
        // `noise` says nothing, so it's gone; `wrapper` collapses into the link it held;
        // `scaffold` stays, because it holds two things.
        let ids: Vec<String> = root.children.iter().map(|n| n.id.to_string()).collect();
        assert_eq!(
            ids,
            ["2", "8", "ff", "5"],
            "`8` is the link, standing where its wrapper did"
        );
        assert_eq!(root.children[0].role, None, "`Unknown` is omitted");
        assert_eq!(root.children[0].children[0].id, Id(0x3));
        assert_eq!(count(&nodes), 7);
    }

    #[test]
    fn a_filter_lifts_matches_past_their_unmatched_ancestors() {
        let nodes = query_all(&QueryFilter {
            query: Query {
                role: Some("button".to_owned()),
                ..Default::default()
            },
            ..Default::default()
        });
        assert_eq!(nodes.len(), 1);
        assert_eq!(
            nodes[0].id,
            Id(0x3),
            "the button, not its scaffold or the root"
        );
        assert!(nodes[0].children.is_empty());
    }

    #[test]
    fn a_container_holding_one_thing_and_saying_nothing_collapses() {
        let nodes = query_all(&QueryFilter::default());
        let ids = |nodes: &[Widget]| -> Vec<String> {
            nodes.iter().map(|node| node.id.to_string()).collect()
        };
        assert!(
            !ids(&nodes[0].children).contains(&"7".to_owned()),
            "`wrapper` had nothing to add and one child to add it to"
        );

        // Unless that role is what the caller asked for — an empty result would be worse.
        let asked = query_all(&QueryFilter {
            query: Query {
                role: Some("genericcontainer".to_owned()),
                ..Default::default()
            },
            ..Default::default()
        });
        assert_eq!(ids(&asked), ["7"]);
    }

    /// ```text
    /// root(Window)
    /// └── panel(GenericContainer)  — the only child, and says nothing, so `root` adopts its children
    ///     ├── ok(Button "OK")
    ///     └── cancel(Button "Cancel")
    /// ```
    fn only_child_tree() -> Tree {
        let mut root = AkNode::new(Role::Window);
        root.set_children(vec![NodeId(0x2)]);
        let mut panel = AkNode::new(Role::GenericContainer);
        panel.set_children(vec![NodeId(0x3), NodeId(0x4)]);
        let mut ok = AkNode::new(Role::Button);
        ok.set_label("OK");
        let mut cancel = AkNode::new(Role::Button);
        cancel.set_label("Cancel");

        Tree::new(
            TreeUpdate {
                nodes: vec![
                    (NodeId(0x1), root),
                    (NodeId(0x2), panel),
                    (NodeId(0x3), ok),
                    (NodeId(0x4), cancel),
                ],
                tree: Some(TreeInfo::new(NodeId(0x1))),
                tree_id: TreeId::ROOT,
                focus: NodeId(0x1),
            },
            false,
        )
    }

    #[test]
    fn a_container_that_is_an_only_child_and_says_nothing_collapses() {
        let ids = |nodes: &[Widget]| -> Vec<String> {
            nodes.iter().map(|node| node.id.to_string()).collect()
        };

        let nodes = query(&only_child_tree(), &QueryFilter::default(), 1.0).expect("no `root`");
        assert_eq!(ids(&nodes), ["1"]);
        assert_eq!(
            ids(&nodes[0].children),
            ["3", "4"],
            "the buttons, standing where their panel did"
        );

        // Unless that role is what the caller asked for.
        let asked = query(
            &only_child_tree(),
            &QueryFilter {
                query: Query {
                    role: Some("genericcontainer".to_owned()),
                    ..Default::default()
                },
                ..Default::default()
            },
            1.0,
        )
        .expect("no `root`");
        assert_eq!(ids(&asked), ["2"]);
    }

    #[test]
    fn an_exclusion_drops_the_matching_node_and_everything_under_it() {
        let nodes = query_all(&QueryFilter {
            exclude: Some(Exclusion {
                query: Query {
                    role: Some("unknown".to_owned()),
                    ..Default::default()
                },
                ..Default::default()
            }),
            ..Default::default()
        });
        // `scaffold` is excluded, so the button and the check box below it go too, though
        // neither matches the exclusion itself. Only the link is left, whose own wrapper is a
        // `GenericContainer` rather than `Unknown`.
        let ids: Vec<String> = nodes[0].children.iter().map(|n| n.id.to_string()).collect();
        assert_eq!(
            ids,
            ["8"],
            "the excluded subtrees took their contents with them"
        );
    }

    /// A mistyped id used to read as "no id": `exclude` quietly matched nothing and left the
    /// subtree in the result, and the caller was never told. [`Id`] rejects it where it is read.
    #[test]
    fn a_malformed_id_is_an_error_rather_than_no_id() {
        let err = serde_json::from_str::<QueryFilter>(r#"{"exclude": {"id": "not-hex"}}"#)
            .expect_err("`not-hex` is not an id");
        assert!(err.to_string().contains("invalid widget id"), "{err}");

        // `root` and the tools' own `id` fields read the same way.
        let err = serde_json::from_str::<QueryFilter>(r#"{"root": "0x12ab"}"#)
            .expect_err("the `0x` prefix is not part of the format");
        assert!(err.to_string().contains("invalid widget id"), "{err}");

        let filter = serde_json::from_str::<QueryFilter>(r#"{"root": "12ab"}"#).expect("plain hex");
        assert_eq!(filter.root, Some(Id(0x12ab)));
    }

    #[test]
    fn a_root_walks_one_subtree_and_a_missing_one_is_an_error() {
        let nodes = query_all(&QueryFilter {
            // `scaffold`, which holds the button and the check box.
            root: Some(Id(0x2)),
            ..Default::default()
        });
        assert_eq!(count(&nodes), 3, "the subtree, not the app");
        assert_eq!(nodes[0].id, Id(0x2));
        assert_eq!(nodes[0].children[0].label.as_deref(), Some("OK"));

        let err = query(
            &test_tree(),
            &QueryFilter {
                root: Some(Id(0xdead)),
                ..Default::default()
            },
            1.0,
        )
        .expect_err("no widget has that id");
        assert!(err.contains("dead"), "{err}");
    }

    #[test]
    fn an_empty_exclusion_excludes_nothing() {
        let all = query_all(&QueryFilter::default());
        let with_empty_exclusion = query_all(&QueryFilter {
            exclude: Some(Exclusion::default()),
            ..Default::default()
        });
        assert_eq!(count(&all), count(&with_empty_exclusion));
    }

    /// Two chains of the same depth under one root, to watch a `limit` spread across both
    /// rather than drain the first: `root → a0 → a1 → a2` and `root → b0 → b1 → b2`.
    fn deep_tree() -> Tree {
        let mut nodes = Vec::new();
        let mut root = AkNode::new(Role::Window);
        root.set_children(vec![NodeId(0xa0), NodeId(0xb0)]);
        nodes.push((NodeId(0x1), root));

        for (branch, base) in [("a", 0xa0), ("b", 0xb0)] {
            for depth in 0..3u64 {
                let id = base + depth;
                let mut node = AkNode::new(Role::Label);
                node.set_label(format!("{branch}{depth}"));
                if depth < 2 {
                    node.set_children(vec![NodeId(id + 1)]);
                }
                nodes.push((NodeId(id), node));
            }
        }

        Tree::new(
            TreeUpdate {
                nodes,
                tree: Some(TreeInfo::new(NodeId(0x1))),
                tree_id: TreeId::ROOT,
                focus: NodeId(0x1),
            },
            false,
        )
    }

    #[test]
    fn a_limit_spreads_across_branches_instead_of_draining_one() {
        // Levels 0..=2 are 1 + 2 + 2 = 5 nodes, so they all fit; level 3 does not.
        let nodes = query(
            &deep_tree(),
            &QueryFilter {
                limit: 5,
                ..Default::default()
            },
            1.0,
        )
        .expect("no `root` to miss");
        assert_eq!(count(&nodes), 5);

        let labels = |nodes: &[Widget]| -> Vec<String> {
            nodes.iter().filter_map(|node| node.label.clone()).collect()
        };
        let top = labels(&nodes[0].children);
        assert_eq!(
            top,
            ["a0", "b0"],
            "both branches, not one branch twice as deep"
        );
        for branch in &nodes[0].children {
            assert_eq!(
                labels(&branch.children).len(),
                1,
                "one level deeper, in both"
            );
            assert_eq!(
                branch.children[0].omitted_children, 1,
                "and each says what it is hiding"
            );
        }
    }

    #[test]
    fn limit_keeps_whole_levels_and_says_what_it_dropped() {
        // The root (1) fits, the level below (4) doesn't, so the root is kept whole and the
        // one node the budget has left goes to its first child.
        let nodes = query_all(&QueryFilter {
            limit: 2,
            ..Default::default()
        });
        assert_eq!(count(&nodes), 2);
        assert_eq!(nodes[0].children.len(), 1);
        assert_eq!(nodes[0].omitted_children, 3, "the link, `ff` and `5`");
        assert_eq!(
            nodes[0].children[0].omitted_children, 2,
            "and both widgets below the kept child"
        );

        // A budget that fits every level leaves the tree alone.
        let whole = query_all(&QueryFilter::default());
        assert_eq!(count(&whole), 7);
        assert!(whole.iter().all(|node| node.omitted_children == 0));

        assert!(
            query_all(&QueryFilter {
                limit: 0,
                ..Default::default()
            })
            .is_empty()
        );
    }

    /// The JSON an agent actually receives.
    ///
    /// Each test above pins one rule; this pins the shape they add up to, so a change to the
    /// output reads as a diff instead of having to be reconstructed from the assertions.
    #[test]
    fn the_widget_tree_json_is_what_an_agent_reads() {
        fn pretty(nodes: &[Widget]) -> String {
            serde_json::to_string_pretty(nodes).expect("serialize")
        }

        insta::assert_snapshot!(
            "widget_tree_unfiltered",
            pretty(&query_all(&QueryFilter::default()))
        );

        // A filter lifts its matches out of the hierarchy, which is the case worth seeing whole.
        insta::assert_snapshot!(
            "widget_tree_filtered",
            pretty(&query_all(&QueryFilter {
                query: Query {
                    role: Some("button".to_owned()),
                    ..Default::default()
                },
                ..Default::default()
            }))
        );
    }

    #[test]
    fn a_tree_node_serializes_without_its_empty_fields() {
        fn keys(node: &Widget) -> Vec<String> {
            let json = serde_json::to_value(node).expect("serialize");
            json.as_object().expect("object").keys().cloned().collect()
        }

        let nodes = query_all(&QueryFilter::default());
        let scaffold = &nodes[0].children[0];
        assert_eq!(
            keys(scaffold),
            ["children", "id"],
            "no label, no value, no role, and every flag false"
        );
        assert_eq!(
            keys(&scaffold.children[0]),
            ["id", "label", "role"],
            "a leaf carries no `children`"
        );
    }
}

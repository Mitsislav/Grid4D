//! Takes (see `docs/reference/manager-take.md`): variations of one document.
//! The **Main** take is the document as it is; every other take overrides
//! some groups of some objects' values (and may pick a camera and its own
//! render settings). The document always holds the *active* take's values;
//! Main's values for what that take overrides are kept in [`Takes`] beside
//! it, so switching is one swap ([`crate::Document::switch_take`]).

use std::collections::BTreeMap;

use slotmap::SlotMap;

use crate::{NodeData, NodeId, ObjectKind, RenderSettings, Tag, Transform};

slotmap::new_key_type! {
    /// Stable, generational handle to a take in [`Takes`].
    pub struct TakeId;
}

/// What a take can override of an object.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum OverrideGroup {
    /// Position, rotation and scale.
    Transform,
    /// Visible in the editor and the renderer, enabled.
    Visibility,
    /// The object's own parameters (not an editable object's points).
    Parameters,
    /// Its tags (Material links among them).
    Tags,
    DisplayColor,
}

impl OverrideGroup {
    pub const ALL: [Self; 5] = [
        Self::Transform,
        Self::Visibility,
        Self::Parameters,
        Self::Tags,
        Self::DisplayColor,
    ];

    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Transform => "Transform",
            Self::Visibility => "Visibility",
            Self::Parameters => "Parameters",
            Self::Tags => "Tags",
            Self::DisplayColor => "Display Color",
        }
    }
}

/// One group's values.
#[derive(Clone, Debug, PartialEq)]
pub enum Override {
    Transform(Transform),
    Visibility { editor: bool, render: bool, enabled: bool },
    Parameters(ObjectKind),
    Tags(Vec<Tag>),
    DisplayColor([f32; 3]),
}

impl Override {
    /// `data`'s values of `group`.
    #[must_use]
    pub fn of(group: OverrideGroup, data: &NodeData) -> Self {
        match group {
            OverrideGroup::Transform => Self::Transform(data.transform),
            OverrideGroup::Visibility => Self::Visibility {
                editor: data.visible_in_editor,
                render: data.visible_in_render,
                enabled: data.enabled,
            },
            OverrideGroup::Parameters => Self::Parameters(data.kind.clone()),
            OverrideGroup::Tags => Self::Tags(data.tags.clone()),
            OverrideGroup::DisplayColor => Self::DisplayColor(data.display_color),
        }
    }

    #[must_use]
    pub const fn group(&self) -> OverrideGroup {
        match self {
            Self::Transform(_) => OverrideGroup::Transform,
            Self::Visibility { .. } => OverrideGroup::Visibility,
            Self::Parameters(_) => OverrideGroup::Parameters,
            Self::Tags(_) => OverrideGroup::Tags,
            Self::DisplayColor(_) => OverrideGroup::DisplayColor,
        }
    }

    /// Writes these values into `data` (Auto Take drops a take's parameters
    /// when an object becomes another kind, so they always fit).
    pub fn write(&self, data: &mut NodeData) {
        match self {
            Self::Transform(transform) => data.transform = *transform,
            Self::Visibility {
                editor,
                render,
                enabled,
            } => {
                data.visible_in_editor = *editor;
                data.visible_in_render = *render;
                data.enabled = *enabled;
            }
            Self::Parameters(kind) => data.kind.clone_from(kind),
            Self::Tags(tags) => data.tags.clone_from(tags),
            Self::DisplayColor(colour) => data.display_color = *colour,
        }
    }
}

/// Whether a take may override `group` of an object of `kind`: an editable
/// object's geometry (points, selections) stays the same in every take.
#[must_use]
pub fn overridable(group: OverrideGroup, kind: &ObjectKind) -> bool {
    group != OverrideGroup::Parameters
        || !matches!(
            kind,
            ObjectKind::Polygon { .. } | ObjectKind::Spline { .. } | ObjectKind::Ffd { .. }
        )
}

/// Groups of values by object, at most one of each group per object.
pub type ObjectOverrides = BTreeMap<NodeId, Vec<Override>>;

/// Sets `value` for `id` in `overrides` (replacing its group's).
pub fn set_override(overrides: &mut ObjectOverrides, id: NodeId, value: Override) {
    let values = overrides.entry(id).or_default();
    match values.iter_mut().find(|v| v.group() == value.group()) {
        Some(existing) => *existing = value,
        None => values.push(value),
    }
}

/// `id`'s value of `group` in `overrides`.
#[must_use]
pub fn get_override(overrides: &ObjectOverrides, id: NodeId, group: OverrideGroup) -> Option<&Override> {
    overrides.get(&id)?.iter().find(|v| v.group() == group)
}

/// Removes `id`'s value of `group`; returns it.
pub fn remove_override(overrides: &mut ObjectOverrides, id: NodeId, group: OverrideGroup) -> Option<Override> {
    let values = overrides.get_mut(&id)?;
    let index = values.iter().position(|v| v.group() == group)?;
    let removed = values.remove(index);
    if values.is_empty() {
        overrides.remove(&id);
    }
    Some(removed)
}

/// A variation of the document.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Take {
    pub name: String,
    /// Its values of the groups it overrides.
    pub objects: ObjectOverrides,
    /// The camera object its renders look through (none: the view's).
    pub camera: Option<NodeId>,
    /// Its own render settings (none: the document's).
    pub render: Option<RenderSettings>,
}

impl Take {
    #[must_use]
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            ..Self::default()
        }
    }

    /// Number of overridden groups.
    #[must_use]
    pub fn override_count(&self) -> usize {
        self.objects.values().map(Vec::len).sum::<usize>() + usize::from(self.render.is_some())
    }
}

/// The document's takes in Take Manager order, the active one, and Main's
/// values for what the active take overrides.
#[derive(Clone, Debug, Default)]
pub struct Takes {
    table: SlotMap<TakeId, Take>,
    order: Vec<TakeId>,
    /// The take the document shows; `None`: Main.
    active: Option<TakeId>,
    /// Main's values of the groups the active take overrides (empty while
    /// Main is active).
    main: ObjectOverrides,
    /// Main's render settings while the active take has its own.
    main_render: Option<RenderSettings>,
    /// Render Settings ▸ Take: the take renders show; `None`: the active one.
    pub render_take: Option<TakeId>,
}

impl PartialEq for Takes {
    fn eq(&self, other: &Self) -> bool {
        self.order == other.order
            && self.order.iter().all(|&id| self.table.get(id) == other.table.get(id))
            && self.active == other.active
            && self.main == other.main
            && self.main_render == other.main_render
            && self.render_take == other.render_take
    }
}

impl Takes {
    #[must_use]
    pub fn len(&self) -> usize {
        self.order.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.order.is_empty()
    }

    #[must_use]
    pub fn get(&self, id: TakeId) -> Option<&Take> {
        self.table.get(id)
    }

    pub fn get_mut(&mut self, id: TakeId) -> Option<&mut Take> {
        self.table.get_mut(id)
    }

    #[must_use]
    pub fn contains(&self, id: TakeId) -> bool {
        self.table.contains_key(id)
    }

    #[must_use]
    pub fn ids(&self) -> &[TakeId] {
        &self.order
    }

    pub fn iter(&self) -> impl Iterator<Item = (TakeId, &Take)> + '_ {
        self.order.iter().filter_map(|&id| self.table.get(id).map(|t| (id, t)))
    }

    /// Appends `take` (not made active).
    pub fn insert(&mut self, take: Take) -> TakeId {
        let id = self.table.insert(take);
        self.order.push(id);
        id
    }

    /// Removes an inactive take; `None` for the active one or a missing one
    /// (switch to Main first).
    pub fn remove(&mut self, id: TakeId) -> Option<Take> {
        if self.active == Some(id) {
            return None;
        }
        let take = self.table.remove(id)?;
        self.order.retain(|&t| t != id);
        if self.render_take == Some(id) {
            self.render_take = None;
        }
        Some(take)
    }

    /// The take the document shows; `None`: Main.
    #[must_use]
    pub const fn active(&self) -> Option<TakeId> {
        self.active
    }

    /// Main's values of what the active take overrides.
    #[must_use]
    pub fn main(&self) -> &ObjectOverrides {
        &self.main
    }

    /// Main's render settings while the active take has its own.
    #[must_use]
    pub fn main_render(&self) -> Option<&RenderSettings> {
        self.main_render.as_ref()
    }

    /// `base`, else `base.1`, `base.2`, ….
    #[must_use]
    pub fn unique_name(&self, base: &str) -> String {
        let taken = |name: &str| name == "Main" || self.iter().any(|(_, t)| t.name == name);
        if !taken(base) {
            return base.to_owned();
        }
        (1..)
            .map(|n| format!("{base}.{n}"))
            .find(|name| !taken(name))
            .unwrap_or_else(|| base.to_owned())
    }

    /// The take renders show: Render Settings ▸ Take if it still exists, else
    /// the active one.
    #[must_use]
    pub fn take_to_render(&self) -> Option<TakeId> {
        self.render_take.filter(|&t| self.contains(t)).or(self.active)
    }

    /// Restores a saved state (file loading): the takes, the active one and
    /// Main's values beside it. The document must hold the active take's values.
    pub fn restore(&mut self, active: Option<TakeId>, main: ObjectOverrides, main_render: Option<RenderSettings>) {
        self.active = active.filter(|&t| self.contains(t));
        if self.active.is_some() {
            self.main = main;
            self.main_render = main_render;
        }
    }

    /// Drops `take`'s override of `group` for `node`. For the active take,
    /// returns Main's value, which the document should show again.
    pub fn clear_override(&mut self, take: TakeId, node: NodeId, group: OverrideGroup) -> Option<Override> {
        remove_override(&mut self.table.get_mut(take)?.objects, node, group)?;
        if self.active == Some(take) {
            remove_override(&mut self.main, node, group)
        } else {
            None
        }
    }

    /// Drops `take`'s own render settings. For the active take, returns
    /// Main's, which the document should use again.
    pub fn clear_render(&mut self, take: TakeId) -> Option<RenderSettings> {
        self.table.get_mut(take)?.render.take()?;
        if self.active == Some(take) {
            self.main_render.take()
        } else {
            None
        }
    }

    pub(crate) fn set_active(&mut self, active: Option<TakeId>) {
        self.active = active;
    }

    pub(crate) fn main_mut(&mut self) -> &mut ObjectOverrides {
        &mut self.main
    }

    pub(crate) fn main_render_mut(&mut self) -> &mut Option<RenderSettings> {
        &mut self.main_render
    }
}

#[cfg(test)]
mod tests {
    use glam::Vec3;

    use super::*;
    use crate::{Document, SceneEdit};

    fn id(doc: &Document, name: &str) -> NodeId {
        doc.scene.iter().find(|(_, n)| n.data.name == name).unwrap().0
    }

    /// An edit made as the editor makes it: applied, then recorded in the
    /// active take; returns the step's inverses.
    fn edit(doc: &mut Document, edit: SceneEdit) -> Vec<SceneEdit> {
        let mut step = vec![doc.apply(edit).unwrap()];
        if let Some(recorded) = doc.record_in_take(&step) {
            step.insert(0, recorded);
        }
        step
    }

    #[test]
    fn edits_in_a_take_become_its_overrides_and_switching_swaps_them() {
        let mut doc = Document::with_demo_scene();
        let cube = id(&doc, "Cube");
        let main_data = doc.scene.get(cube).unwrap().data.clone();
        let main_render = doc.render.clone();
        let take = doc.takes.insert(Take::new("Red Cube"));
        doc.apply(SceneEdit::SwitchTake(Some(take))).unwrap();
        assert_eq!(doc.takes.active(), Some(take));

        // In the take: move the cube, recolour it, hide it, change its size
        // and the render size.
        let mut data = main_data.clone();
        data.transform.position = Vec3::new(50.0, 0.0, 0.0);
        data.display_color = [1.0, 0.0, 0.0];
        data.visible_in_render = false;
        data.kind = ObjectKind::Cube {
            size: Vec3::splat(10.0),
            segments: [1, 1, 1],
        };
        data.name = "Renamed".to_owned();
        let step = edit(&mut doc, SceneEdit::SetData { id: cube, data });
        assert_eq!(step.len(), 2, "the edit and its recording, one step");
        let mut render = doc.render.clone();
        render.width = 320;
        edit(&mut doc, SceneEdit::SetRender(render));
        let recorded = &doc.takes.get(take).unwrap().objects[&cube];
        let groups: Vec<OverrideGroup> = recorded.iter().map(Override::group).collect();
        assert_eq!(
            groups,
            [
                OverrideGroup::Transform,
                OverrideGroup::Visibility,
                OverrideGroup::Parameters,
                OverrideGroup::DisplayColor
            ],
            "only the groups that changed; names are not a take's"
        );
        assert_eq!(doc.takes.get(take).unwrap().override_count(), 5);

        // Back to Main: Main's values (the name stays, it is not overridden).
        doc.apply(SceneEdit::SwitchTake(None)).unwrap();
        let back = &doc.scene.get(cube).unwrap().data;
        assert_eq!(back.transform, main_data.transform);
        assert_eq!(back.display_color, main_data.display_color);
        assert!(back.visible_in_render);
        assert_eq!(back.kind, main_data.kind);
        assert_eq!(back.name, "Renamed");
        assert_eq!(doc.render, main_render);
        assert!(doc.takes.main().is_empty(), "nothing kept beside Main");

        // In Main, edits are Main's: nothing is recorded.
        let mut moved = back.clone();
        moved.transform.position.y = 400.0;
        assert_eq!(edit(&mut doc, SceneEdit::SetData { id: cube, data: moved }).len(), 1);

        // Into the take again: its values, over Main's new ones.
        let inverse = doc.apply(SceneEdit::SwitchTake(Some(take))).unwrap();
        let shown = &doc.scene.get(cube).unwrap().data;
        assert_eq!(shown.transform.position, Vec3::new(50.0, 0.0, 0.0));
        assert_eq!(doc.render.width, 320);
        // Undoing the switch restores Main exactly.
        doc.apply(inverse).unwrap();
        assert_eq!(doc.scene.get(cube).unwrap().data.transform.position.y, 400.0);
        assert_eq!(doc.render, main_render);
    }

    #[test]
    fn the_table_and_its_names() {
        let mut takes = Takes::default();
        let a = takes.insert(Take::new(takes.unique_name("Main")));
        assert_eq!(takes.get(a).unwrap().name, "Main.1", "Main is taken");
        let b = takes.insert(Take::new(takes.unique_name("Main.1")));
        assert_eq!(takes.get(b).unwrap().name, "Main.1.1");
        takes.render_take = Some(b);
        assert_eq!(takes.take_to_render(), Some(b));
        assert!(takes.remove(b).is_some());
        assert_eq!(takes.take_to_render(), None, "renders show the active take again");
        assert_eq!(takes.ids(), &[a]);
    }
}

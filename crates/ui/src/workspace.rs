//! The docked panes (see `docs/reference/workspace-layouts.md`): the viewport
//! and the managers in a tree of tabs and splits (`egui_tiles`), which the user
//! rearranges by dragging tabs; three preset layouts; the arrangement saved by
//! the binary.

use egui::{Color32, Stroke, Ui, Visuals, WidgetText};
use egui_tiles::{Behavior, Container, SimplificationOptions, TabState, Tile, TileId, Tiles, Tree, UiResponse};
use grid4d_scene::Document;
use serde::{Deserialize, Serialize};

use crate::{
    EditorState,
    panels::{
        AttributesPanel, CoordinatesPanel, FCurvePanel, MaterialsPanel, OutlinerPanel, TimelinePanel, ViewportPanel,
    },
    theme::{self, metrics, palette as p},
};

/// Where eframe storage keeps the arrangement.
pub const STORAGE_KEY: &str = "grid4d.workspace";

/// A docked part of the window.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Pane {
    Viewport,
    Objects,
    Takes,
    Attributes,
    Coordinates,
    Layers,
    Timeline,
    FCurve,
    Materials,
}

impl Pane {
    pub const ALL: [Self; 9] = [
        Self::Viewport,
        Self::Objects,
        Self::Takes,
        Self::Attributes,
        Self::Coordinates,
        Self::Layers,
        Self::Timeline,
        Self::FCurve,
        Self::Materials,
    ];

    /// Its tab's title.
    #[must_use]
    pub const fn title(self) -> &'static str {
        match self {
            Self::Viewport => "Viewport",
            Self::Objects => "Objects",
            Self::Takes => "Takes",
            Self::Attributes => "Attributes",
            Self::Coordinates => "Coordinates",
            Self::Layers => "Layers",
            Self::Timeline => "Timeline",
            Self::FCurve => "F-Curve",
            Self::Materials => "Materials",
        }
    }

    /// Its name in the Window menu.
    #[must_use]
    pub const fn window_label(self) -> &'static str {
        match self {
            Self::Viewport => "Viewport",
            Self::Objects => "Object Manager",
            Self::Takes => "Take Manager",
            Self::Attributes => "Attribute Manager",
            Self::Coordinates => "Coordinate Manager",
            Self::Layers => "Layer Manager",
            Self::Timeline => "Timeline",
            Self::FCurve => "F-Curve Editor",
            Self::Materials => "Material Manager",
        }
    }

    /// The panes it usually sits beside (reopened as their tab).
    const fn neighbours(self) -> &'static [Self] {
        match self {
            Self::Viewport => &[],
            Self::Objects => &[Self::Takes, Self::Layers],
            Self::Takes => &[Self::Objects, Self::Layers],
            Self::Attributes => &[Self::Coordinates, Self::Layers],
            Self::Coordinates => &[Self::Attributes, Self::Layers],
            Self::Layers => &[Self::Attributes, Self::Objects],
            Self::Timeline => &[Self::FCurve, Self::Materials],
            Self::FCurve => &[Self::Timeline, Self::Materials],
            Self::Materials => &[Self::Timeline, Self::FCurve, Self::Objects],
        }
    }

    const fn bit(self) -> u16 {
        1 << self as u16
    }
}

/// A preset arrangement.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Layout {
    #[default]
    Standard,
    Modeling,
    Animate,
}

impl Layout {
    pub const ALL: [Self; 3] = [Self::Standard, Self::Modeling, Self::Animate];

    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Standard => "Standard",
            Self::Modeling => "Modeling",
            Self::Animate => "Animate",
        }
    }
}

/// What the panes did last frame, and what the rest of the editor asks of them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PaneState {
    shown: u16,
    /// The pane under the pointer.
    pub hovered: Option<Pane>,
    /// Bring this pane's tab to the front (opening it if it is closed).
    pub show: Option<Pane>,
    /// Arrange the panes this way.
    pub layout: Option<Layout>,
    /// Put the current layout back as it was (Window ▸ Reset Layout).
    pub reset: bool,
    /// The layout the arrangement came from.
    pub current: Layout,
}

impl Default for PaneState {
    /// As the Standard layout starts: the viewport, the Object and Attribute
    /// Managers and the timeline in view.
    fn default() -> Self {
        Self {
            shown: Pane::Viewport.bit() | Pane::Objects.bit() | Pane::Attributes.bit() | Pane::Timeline.bit(),
            hovered: None,
            show: None,
            layout: None,
            reset: false,
            current: Layout::Standard,
        }
    }
}

impl PaneState {
    /// `pane` was drawn last frame (it is open and its tab in front).
    #[must_use]
    pub const fn is_shown(&self, pane: Pane) -> bool {
        self.shown & pane.bit() != 0
    }

    /// Records `pane` as drawn (the workspace does this as it draws it).
    pub fn mark_shown(&mut self, pane: Pane) {
        self.shown |= pane.bit();
    }
}

/// The docked panels themselves.
#[derive(Debug)]
pub struct Panels {
    pub viewport: ViewportPanel,
    pub outliner: OutlinerPanel,
    pub attributes: AttributesPanel,
    pub coordinates: CoordinatesPanel,
    pub timeline: TimelinePanel,
    pub fcurve: FCurvePanel,
    pub materials: MaterialsPanel,
}

impl Panels {
    #[must_use]
    pub fn new(viewport: ViewportPanel) -> Self {
        Self {
            viewport,
            outliner: OutlinerPanel::default(),
            attributes: AttributesPanel::default(),
            coordinates: CoordinatesPanel::default(),
            timeline: TimelinePanel::default(),
            fcurve: FCurvePanel::default(),
            materials: MaterialsPanel,
        }
    }
}

/// The arrangement of the panes.
#[derive(Debug, Serialize, Deserialize)]
pub struct Workspace {
    tree: Tree<Pane>,
    layout: Layout,
}

impl Default for Workspace {
    fn default() -> Self {
        Self::new(Layout::Standard)
    }
}

impl Workspace {
    /// The panes arranged as `layout`.
    #[must_use]
    pub fn new(layout: Layout) -> Self {
        Self {
            tree: preset(layout),
            layout,
        }
    }

    /// The layout the arrangement came from.
    #[must_use]
    pub const fn layout(&self) -> Layout {
        self.layout
    }

    /// Rearranges the panes as `layout`.
    pub fn use_layout(&mut self, layout: Layout) {
        *self = Self::new(layout);
    }

    /// `true` if `pane` is open (in the tree).
    #[must_use]
    pub fn contains(&self, pane: Pane) -> bool {
        self.tree.tiles.find_pane(&pane).is_some()
    }

    /// The arrangement made sound: a pane found twice (or none at all) gives
    /// the preset back; a missing Viewport is added. Other missing panes were
    /// closed and stay closed (the Window menu reopens them).
    #[must_use]
    pub fn repaired(mut self) -> Self {
        let mut seen = 0u16;
        for tile in self.tree.tiles.tiles() {
            if let Tile::Pane(pane) = tile {
                if seen & pane.bit() != 0 {
                    return Self::new(self.layout);
                }
                seen |= pane.bit();
            }
        }
        if self.tree.root().is_none() {
            return Self::new(self.layout);
        }
        if !self.contains(Pane::Viewport) {
            self.insert(Pane::Viewport);
        }
        self
    }

    /// Brings `pane`'s tab to the front, opening it beside its neighbours if it
    /// is closed.
    pub fn show_pane(&mut self, pane: Pane) {
        if !self.contains(pane) {
            self.insert(pane);
        }
        let Some(tile) = self.tree.tiles.find_pane(&pane) else {
            return;
        };
        self.tree.make_active(|id, _| id == tile);
    }

    /// Adds `pane` as a tab beside its first open neighbour, else beside the
    /// whole arrangement.
    fn insert(&mut self, pane: Pane) {
        let tiles = &mut self.tree.tiles;
        let group = pane
            .neighbours()
            .iter()
            .filter_map(|n| tiles.find_pane(n))
            .find_map(|tile| tiles.parent_of(tile))
            .filter(|&parent| matches!(tiles.get(parent), Some(Tile::Container(Container::Tabs(_)))));
        let id = tiles.insert_pane(pane);
        if let Some(group) = group
            && let Some(Tile::Container(Container::Tabs(tabs))) = tiles.get_mut(group)
        {
            tabs.add_child(id);
            tabs.set_active(id);
            return;
        }
        let tabs = tiles.insert_tab_tile(vec![id]);
        let root = match self.tree.root {
            Some(root) => tiles.insert_horizontal_tile(vec![root, tabs]),
            None => tabs,
        };
        self.tree.root = Some(root);
    }

    /// Draws the panes (each through its panel) after handling the editor's
    /// requests, and records which were drawn and which is under the pointer.
    pub fn show(&mut self, ui: &mut Ui, panels: &mut Panels, doc: &mut Document, editor: &mut EditorState) {
        let requests = &mut editor.panes;
        if let Some(layout) = requests.layout.take() {
            self.use_layout(layout);
        }
        if std::mem::take(&mut requests.reset) {
            self.use_layout(self.layout);
        }
        if let Some(pane) = requests.show.take() {
            self.show_pane(pane);
        }
        requests.current = self.layout;
        requests.shown = 0;
        requests.hovered = None;
        let viewport_tabs = self
            .tree
            .tiles
            .find_pane(&Pane::Viewport)
            .and_then(|tile| self.tree.tiles.parent_of(tile));
        let mut dispatch = Dispatch {
            panels,
            doc,
            editor,
            viewport_tabs,
        };
        self.tree.ui(&mut dispatch, ui);
    }
}

/// Draws each pane through its panel; styles the tabs with the palette.
struct Dispatch<'a> {
    panels: &'a mut Panels,
    doc: &'a mut Document,
    editor: &'a mut EditorState,
    /// The viewport's tab group (no tab bar while it is alone there).
    viewport_tabs: Option<TileId>,
}

impl Behavior<Pane> for Dispatch<'_> {
    fn pane_ui(&mut self, ui: &mut Ui, _tile_id: TileId, pane: &mut Pane) -> UiResponse {
        let (panels, doc, editor) = (&mut *self.panels, &mut *self.doc, &mut *self.editor);
        editor.panes.mark_shown(*pane);
        if ui.rect_contains_pointer(ui.max_rect()) {
            editor.panes.hovered = Some(*pane);
        }
        match *pane {
            Pane::Viewport => panels.viewport.show(ui, doc, editor),
            other => {
                theme::panel_frame().show(ui, |ui| {
                    ui.set_min_size(ui.available_size());
                    match other {
                        Pane::Objects => panels.outliner.show(ui, doc, editor),
                        Pane::Attributes => panels.attributes.show(ui, doc, editor),
                        Pane::Coordinates => panels.coordinates.show(ui, doc, editor),
                        Pane::Timeline => panels.timeline.show(ui, doc, editor),
                        Pane::FCurve => panels.fcurve.show(ui, doc, editor),
                        Pane::Materials => panels.materials.show(ui, doc, editor),
                        Pane::Takes => planned(ui, "Takes come with step 12.5"),
                        Pane::Layers => planned(ui, "Layers come with step 12.4"),
                        Pane::Viewport => {}
                    }
                });
            }
        }
        UiResponse::None
    }

    fn tab_title_for_pane(&mut self, pane: &Pane) -> WidgetText {
        pane.title().into()
    }

    fn is_tab_closable(&self, tiles: &Tiles<Pane>, tile_id: TileId) -> bool {
        tiles.get_pane(&tile_id).is_some_and(|&pane| pane != Pane::Viewport)
    }

    fn is_tab_bar_visible(&self, tile_id: TileId, tabs: &egui_tiles::Tabs) -> bool {
        Some(tile_id) != self.viewport_tabs || tabs.children.len() > 1
    }

    fn tab_bar_height(&self, _style: &egui::Style) -> f32 {
        metrics::HEADER_HEIGHT
    }

    fn gap_width(&self, _style: &egui::Style) -> f32 {
        2.0
    }

    fn min_size(&self) -> f32 {
        60.0
    }

    fn simplification_options(&self) -> SimplificationOptions {
        SimplificationOptions {
            all_panes_must_have_tabs: true,
            ..SimplificationOptions::default()
        }
    }

    fn tab_bar_color(&self, _visuals: &Visuals) -> Color32 {
        p::BG_HEADER
    }

    fn tab_bg_color(&self, _visuals: &Visuals, _tiles: &Tiles<Pane>, _tile_id: TileId, state: &TabState) -> Color32 {
        if state.active { p::BG_PANEL } else { p::BG_HEADER }
    }

    fn tab_outline_stroke(
        &self,
        _visuals: &Visuals,
        _tiles: &Tiles<Pane>,
        _tile_id: TileId,
        _state: &TabState,
    ) -> Stroke {
        Stroke::NONE
    }

    fn tab_bar_hline_stroke(&self, _visuals: &Visuals) -> Stroke {
        Stroke::NONE
    }

    fn tab_text_color(&self, _visuals: &Visuals, _tiles: &Tiles<Pane>, _tile_id: TileId, state: &TabState) -> Color32 {
        if state.active { p::TEXT_STRONG } else { p::TEXT_DIM }
    }

    fn resize_stroke(&self, _style: &egui::Style, state: egui_tiles::ResizeState) -> Stroke {
        match state {
            egui_tiles::ResizeState::Idle => Stroke::new(2.0, p::BG_WELL),
            egui_tiles::ResizeState::Hovering | egui_tiles::ResizeState::Dragging => Stroke::new(2.0, p::ACCENT),
        }
    }

    fn drag_preview_color(&self, _visuals: &Visuals) -> Color32 {
        p::ACCENT.gamma_multiply(0.25)
    }
}

/// A pane whose manager is still to come.
fn planned(ui: &mut Ui, text: &str) {
    ui.add_space(12.0);
    ui.vertical_centered(|ui| ui.label(egui::RichText::new(text).color(p::TEXT_DIM)));
}

/// A tab group of `panes`, the first in front.
fn tabs(tiles: &mut Tiles<Pane>, panes: &[Pane]) -> TileId {
    let children = panes.iter().map(|&pane| tiles.insert_pane(pane)).collect();
    tiles.insert_tab_tile(children)
}

/// A row (or column) of `children`, sized by their shares.
fn linear(tiles: &mut Tiles<Pane>, horizontal: bool, children: &[(TileId, f32)]) -> TileId {
    let ids = children.iter().map(|&(id, _)| id).collect();
    let id = if horizontal {
        tiles.insert_horizontal_tile(ids)
    } else {
        tiles.insert_vertical_tile(ids)
    };
    if let Some(Tile::Container(Container::Linear(row))) = tiles.get_mut(id) {
        for &(child, share) in children {
            row.shares.set_share(child, share);
        }
    }
    id
}

/// The tree of a preset layout.
fn preset(layout: Layout) -> Tree<Pane> {
    use Pane::{Attributes, Coordinates, FCurve, Layers, Materials, Objects, Takes, Timeline, Viewport};
    let mut tiles = Tiles::default();
    let t = &mut tiles;
    let root = match layout {
        Layout::Standard => {
            let viewport = tabs(t, &[Viewport]);
            let bottom = tabs(t, &[Timeline, FCurve, Materials]);
            let left = linear(t, false, &[(viewport, 0.8), (bottom, 0.2)]);
            let objects = tabs(t, &[Objects, Takes]);
            let attributes = tabs(t, &[Attributes, Coordinates, Layers]);
            let right = linear(t, false, &[(objects, 0.47), (attributes, 0.53)]);
            linear(t, true, &[(left, 0.64), (right, 0.36)])
        }
        Layout::Modeling => {
            let viewport = tabs(t, &[Viewport]);
            let bottom = tabs(t, &[Timeline, FCurve]);
            let left = linear(t, false, &[(viewport, 0.86), (bottom, 0.14)]);
            let objects = tabs(t, &[Objects, Takes, Layers, Materials]);
            let attributes = tabs(t, &[Attributes]);
            let coordinates = tabs(t, &[Coordinates]);
            let managers = linear(t, true, &[(attributes, 0.55), (coordinates, 0.45)]);
            let right = linear(t, false, &[(objects, 0.4), (managers, 0.6)]);
            linear(t, true, &[(left, 0.58), (right, 0.42)])
        }
        Layout::Animate => {
            let viewport = tabs(t, &[Viewport]);
            let bottom = tabs(t, &[FCurve, Timeline, Materials]);
            let left = linear(t, false, &[(viewport, 0.55), (bottom, 0.45)]);
            let objects = tabs(t, &[Objects, Takes, Layers]);
            let attributes = tabs(t, &[Attributes, Coordinates]);
            let right = linear(t, false, &[(objects, 0.5), (attributes, 0.5)]);
            linear(t, true, &[(left, 0.7), (right, 0.3)])
        }
    };
    Tree::new(egui::Id::unique("grid4d.workspace"), root, tiles)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn panes(workspace: &Workspace) -> Vec<Pane> {
        let mut found: Vec<Pane> = workspace
            .tree
            .tiles
            .tiles()
            .filter_map(|tile| match tile {
                Tile::Pane(pane) => Some(*pane),
                Tile::Container(_) => None,
            })
            .collect();
        found.sort_by_key(|&p| p as u8);
        found
    }

    #[test]
    fn every_layout_holds_every_pane_once() {
        for layout in Layout::ALL {
            assert_eq!(panes(&Workspace::new(layout)), Pane::ALL, "{layout:?}");
        }
    }

    #[test]
    fn a_closed_pane_reopens_beside_its_neighbours_and_a_broken_tree_is_repaired() {
        let mut workspace = Workspace::default();
        let coordinates = workspace.tree.tiles.find_pane(&Pane::Coordinates).unwrap();
        let group = workspace.tree.tiles.parent_of(coordinates).unwrap();
        workspace.tree.remove_recursively(coordinates);
        assert!(!workspace.contains(Pane::Coordinates));
        workspace.show_pane(Pane::Coordinates);
        let back = workspace.tree.tiles.find_pane(&Pane::Coordinates).unwrap();
        assert_eq!(
            workspace.tree.tiles.parent_of(back),
            Some(group),
            "beside the Attribute Manager"
        );
        let Some(Tile::Container(Container::Tabs(tabs))) = workspace.tree.tiles.get(group) else {
            panic!("not a tab group")
        };
        assert!(tabs.is_active(back), "in front");

        // A saved arrangement keeps its closed panes closed but always gets
        // the Viewport back; one with a pane twice becomes the preset.
        let mut missing = Workspace::new(Layout::Animate);
        for pane in [Pane::Viewport, Pane::Materials] {
            let tile = missing.tree.tiles.find_pane(&pane).unwrap();
            missing.tree.remove_recursively(tile);
        }
        let repaired = missing.repaired();
        assert!(repaired.contains(Pane::Viewport));
        assert!(!repaired.contains(Pane::Materials), "closed stays closed");
        assert_eq!(panes(&repaired).len(), Pane::ALL.len() - 1);
        assert_eq!(repaired.layout(), Layout::Animate);
        let mut twice = Workspace::default();
        let extra = twice.tree.tiles.insert_pane(Pane::Objects);
        let root = twice.tree.root().unwrap();
        let both = twice.tree.tiles.insert_horizontal_tile(vec![root, extra]);
        twice.tree.root = Some(both);
        assert_eq!(panes(&twice.repaired()), Pane::ALL);
    }

    #[test]
    fn requests_switch_layouts_and_show_panes_and_drawing_marks_them() {
        let mut workspace = Workspace::default();
        let mut panels = Panels::new(ViewportPanel::new(None));
        let mut doc = Document::with_demo_scene();
        let mut editor = EditorState::default();
        let ctx = egui::Context::default();
        let mut frame = |workspace: &mut Workspace, editor: &mut EditorState| {
            ctx.run_ui(egui::RawInput::default(), |ui| {
                workspace.show(ui, &mut panels, &mut doc, editor);
            })
            .drop_without_applying_deltas();
        };
        frame(&mut workspace, &mut editor);
        for pane in [Pane::Viewport, Pane::Objects, Pane::Attributes, Pane::Timeline] {
            assert!(editor.panes.is_shown(pane), "{pane:?}");
        }
        assert!(!editor.panes.is_shown(Pane::FCurve));
        editor.panes.show = Some(Pane::FCurve);
        frame(&mut workspace, &mut editor);
        assert!(editor.panes.is_shown(Pane::FCurve) && !editor.panes.is_shown(Pane::Timeline));
        editor.panes.layout = Some(Layout::Modeling);
        frame(&mut workspace, &mut editor);
        assert_eq!(
            (workspace.layout(), editor.panes.current),
            (Layout::Modeling, Layout::Modeling)
        );
        assert!(editor.panes.is_shown(Pane::Coordinates) && editor.panes.is_shown(Pane::Attributes));
    }

    /// Where egui's accessibility tree puts the widgets labelled `label`.
    fn widgets(output: &egui::FullOutput, label: &str) -> Vec<egui::Pos2> {
        let Some(update) = &output.platform_output.accesskit_update else {
            return Vec::new();
        };
        update
            .nodes
            .iter()
            .filter(|(_, node)| node.label() == Some(label))
            .filter_map(|(_, node)| node.bounds())
            .map(|b| egui::pos2(((b.x0 + b.x1) / 2.0) as f32, ((b.y0 + b.y1) / 2.0) as f32))
            .collect()
    }

    #[test]
    fn clicking_a_tab_fronts_it_and_its_cross_closes_it() {
        let mut workspace = Workspace::default();
        let mut panels = Panels::new(ViewportPanel::new(None));
        let mut doc = Document::with_demo_scene();
        let mut editor = EditorState::default();
        let ctx = egui::Context::default();
        ctx.enable_accesskit();
        let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1280.0, 800.0));
        let mut frame = |workspace: &mut Workspace, editor: &mut EditorState, events: Vec<egui::Event>| {
            let input = egui::RawInput {
                screen_rect: Some(screen),
                events,
                ..egui::RawInput::default()
            };
            let mut output = ctx.run_ui(input, |ui| workspace.show(ui, &mut panels, &mut doc, editor));
            output.textures_delta.clear();
            output
        };
        let click = |pos: egui::Pos2| {
            let button = |pressed| egui::Event::PointerButton {
                pos,
                button: egui::PointerButton::Primary,
                pressed,
                modifiers: egui::Modifiers::NONE,
            };
            vec![
                vec![egui::Event::PointerMoved(pos)],
                vec![button(true)],
                vec![button(false)],
                Vec::new(),
            ]
        };
        let output = frame(&mut workspace, &mut editor, Vec::new());
        assert!(!editor.panes.is_shown(Pane::Coordinates));
        let tab = widgets(&output, "Coordinates");
        assert_eq!(tab.len(), 1, "one Coordinates tab");
        for events in click(tab[0]) {
            frame(&mut workspace, &mut editor, events);
        }
        assert!(editor.panes.is_shown(Pane::Coordinates), "clicked to the front");

        // The front tab's cross is the one nearest to the right of its title.
        let output = frame(&mut workspace, &mut editor, Vec::new());
        let tab = widgets(&output, "Coordinates")[0];
        let cross = widgets(&output, "Close")
            .into_iter()
            .filter(|c| (c.y - tab.y).abs() < 2.0 && c.x > tab.x)
            .min_by(|a, b| a.x.total_cmp(&b.x))
            .unwrap();
        for events in click(cross) {
            frame(&mut workspace, &mut editor, events);
        }
        assert!(!workspace.contains(Pane::Coordinates), "closed");
    }

    #[test]
    fn the_arrangement_survives_saving() {
        let mut workspace = Workspace::new(Layout::Animate);
        let coordinates = workspace.tree.tiles.find_pane(&Pane::Coordinates).unwrap();
        workspace.tree.remove_recursively(coordinates);
        let text = serde_json::to_string(&workspace).unwrap();
        let back = serde_json::from_str::<Workspace>(&text).unwrap().repaired();
        assert_eq!(back.layout(), Layout::Animate);
        assert!(!back.contains(Pane::Coordinates), "closed stays closed");
        assert_eq!(panes(&back), panes(&workspace));
    }

    #[test]
    fn idle_frames_in_every_layout_leave_the_document_alone() {
        let mut panels = Panels::new(ViewportPanel::new(None));
        let mut doc = Document::with_demo_scene();
        let mut editor = EditorState::default();
        editor.selection.set_single(doc.scene.roots()[0]);
        let ctx = egui::Context::default();
        for layout in Layout::ALL {
            let mut workspace = Workspace::new(layout);
            let mut frame = |doc: &mut Document, editor: &mut EditorState| {
                ctx.run_ui(egui::RawInput::default(), |ui| {
                    workspace.show(ui, &mut panels, doc, editor)
                })
                .drop_without_applying_deltas();
                crate::commands::apply_pending(doc, editor, &mut panels.viewport);
            };
            frame(&mut doc, &mut editor);
            frame(&mut doc, &mut editor);
            let revision = doc.scene.revision();
            for _ in 0..3 {
                frame(&mut doc, &mut editor);
            }
            assert_eq!(doc.scene.revision(), revision, "{layout:?}: a pane wrote the scene");
        }
    }
}

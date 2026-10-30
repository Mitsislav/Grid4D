//! Layers (see `docs/reference/manager-layer.md`): named, coloured groups of
//! objects with switches, kept once per document in an ordered table. An
//! object names its layer in `NodeData::layer`; children without one are on
//! their parent's.

use slotmap::SlotMap;

slotmap::new_key_type! {
    /// Stable, generational handle to a layer in [`Layers`]: a removed
    /// layer's id never names another one.
    pub struct LayerId;
}

/// What a layer lets its objects do.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[expect(
    clippy::struct_excessive_bools,
    reason = "one switch per column of the Layer Manager"
)]
pub struct LayerSwitches {
    /// While any layer is soloed, only soloed layers' objects show.
    pub solo: bool,
    /// Shown in the views.
    pub view: bool,
    /// Shown in renders.
    pub render: bool,
    /// Listed in the Object Manager.
    pub manager: bool,
    /// Not pickable or movable in the views.
    pub locked: bool,
    /// Animated (off: shown at no frame).
    pub animation: bool,
    /// Generators build (off: they act as switched off).
    pub generators: bool,
    /// Deformers deform (off: they do nothing).
    pub deformers: bool,
}

impl Default for LayerSwitches {
    fn default() -> Self {
        Self {
            solo: false,
            view: true,
            render: true,
            manager: true,
            locked: false,
            animation: true,
            generators: true,
            deformers: true,
        }
    }
}

/// A layer: a name, a colour (linear RGB) and its switches.
#[derive(Clone, Debug, PartialEq)]
pub struct Layer {
    pub name: String,
    pub color: [f32; 3],
    pub switches: LayerSwitches,
}

impl Layer {
    /// Colours new layers take in turn (linear RGB: red, green, blue, yellow,
    /// magenta, cyan, orange, violet).
    pub const COLORS: [[f32; 3]; 8] = [
        [0.80, 0.10, 0.08],
        [0.18, 0.62, 0.10],
        [0.08, 0.25, 0.85],
        [0.90, 0.75, 0.05],
        [0.75, 0.08, 0.60],
        [0.05, 0.62, 0.70],
        [0.95, 0.35, 0.03],
        [0.40, 0.15, 0.85],
    ];

    #[must_use]
    pub fn new(name: impl Into<String>, color: [f32; 3]) -> Self {
        Self {
            name: name.into(),
            color,
            switches: LayerSwitches::default(),
        }
    }
}

/// The document's layers in Layer Manager order.
#[derive(Clone, Debug, Default)]
pub struct Layers {
    table: SlotMap<LayerId, Layer>,
    order: Vec<LayerId>,
}

impl PartialEq for Layers {
    fn eq(&self, other: &Self) -> bool {
        self.order == other.order && self.order.iter().all(|&id| self.table.get(id) == other.table.get(id))
    }
}

impl Layers {
    #[must_use]
    pub fn len(&self) -> usize {
        self.order.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.order.is_empty()
    }

    #[must_use]
    pub fn get(&self, id: LayerId) -> Option<&Layer> {
        self.table.get(id)
    }

    pub fn get_mut(&mut self, id: LayerId) -> Option<&mut Layer> {
        self.table.get_mut(id)
    }

    #[must_use]
    pub fn contains(&self, id: LayerId) -> bool {
        self.table.contains_key(id)
    }

    /// Ids in Layer Manager order.
    #[must_use]
    pub fn ids(&self) -> &[LayerId] {
        &self.order
    }

    /// Layers in Layer Manager order.
    pub fn iter(&self) -> impl Iterator<Item = (LayerId, &Layer)> + '_ {
        self.order.iter().filter_map(|&id| self.table.get(id).map(|l| (id, l)))
    }

    /// Appends `layer`.
    pub fn insert(&mut self, layer: Layer) -> LayerId {
        let id = self.table.insert(layer);
        self.order.push(id);
        id
    }

    /// Removes the layer; `None` if there was none.
    pub fn remove(&mut self, id: LayerId) -> Option<Layer> {
        let layer = self.table.remove(id)?;
        self.order.retain(|&l| l != id);
        Some(layer)
    }

    /// Position in Layer Manager order.
    #[must_use]
    pub fn index_of(&self, id: LayerId) -> Option<usize> {
        self.order.iter().position(|&l| l == id)
    }

    /// `base` if no layer has that name, else `base.1`, `base.2`, ….
    #[must_use]
    pub fn unique_name(&self, base: &str) -> String {
        let taken = |name: &str| self.iter().any(|(_, l)| l.name == name);
        if !taken(base) {
            return base.to_owned();
        }
        (1..)
            .map(|n| format!("{base}.{n}"))
            .find(|name| !taken(name))
            .unwrap_or_else(|| base.to_owned())
    }

    /// A new layer named after `base` with the next colour in turn.
    #[must_use]
    pub fn next_layer(&self, base: &str) -> Layer {
        let color = Layer::COLORS[self.table.len() % Layer::COLORS.len()];
        Layer::new(self.unique_name(base), color)
    }

    /// The switches of `layer` (a missing layer, or none: the defaults).
    #[must_use]
    pub fn switches(&self, layer: Option<LayerId>) -> LayerSwitches {
        layer
            .and_then(|id| self.table.get(id))
            .map_or_else(LayerSwitches::default, |l| l.switches)
    }

    /// Whether any layer is soloed.
    #[must_use]
    pub fn solo_active(&self) -> bool {
        self.table.values().any(|l| l.switches.solo)
    }

    /// Whether objects whose (effective) layer is `layer` show in the views
    /// (`render` false) or in renders.
    #[must_use]
    pub fn shows(&self, layer: Option<LayerId>, render: bool) -> bool {
        let switches = self.switches(layer);
        let soloed = !self.solo_active() || layer.is_some_and(|_| switches.solo);
        soloed && if render { switches.render } else { switches.view }
    }

    /// [`Self::shows`] for an object inside a shown one: `own` is its own
    /// layer, and without one it shows like its parent.
    #[must_use]
    pub fn shows_own(&self, own: Option<LayerId>, render: bool) -> bool {
        own.filter(|&l| self.contains(l))
            .is_none_or(|l| self.shows(Some(l), render))
    }

    /// Whether every layer animates (nothing to check per object).
    #[must_use]
    pub fn all_animate(&self) -> bool {
        self.table.values().all(|l| l.switches.animation)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ordered_table_with_unique_names_colours_in_turn_and_stable_ids() {
        let mut layers = Layers::default();
        let a = layers.insert(layers.next_layer("Layer"));
        let b = layers.insert(layers.next_layer("Layer"));
        assert_eq!(layers.get(b).unwrap().name, "Layer.1");
        assert_ne!(layers.get(a).unwrap().color, layers.get(b).unwrap().color);
        assert_eq!(layers.ids(), &[a, b]);
        let copy = layers.clone();
        layers.remove(a);
        let c = layers.insert(Layer::new("Layer", [1.0; 3]));
        assert_ne!(c, a, "a removed id never names another layer");
        assert_eq!(copy.ids(), &[a, b]);
        assert_ne!(copy, layers);
    }

    #[test]
    fn view_render_and_solo_decide_what_shows() {
        let mut layers = Layers::default();
        let lights = layers.insert(Layer::new("Lights", [1.0; 3]));
        let set = layers.insert(Layer::new("Set", [1.0; 3]));
        let gone = layers.insert(Layer::new("Gone", [1.0; 3]));
        layers.remove(gone);
        for layer in [None, Some(lights), Some(set), Some(gone)] {
            assert!(layers.shows(layer, false) && layers.shows(layer, true));
        }
        layers.get_mut(lights).unwrap().switches.view = false;
        assert!(!layers.shows(Some(lights), false) && layers.shows(Some(lights), true));
        layers.get_mut(set).unwrap().switches.render = false;
        assert!(layers.shows(Some(set), false) && !layers.shows(Some(set), true));

        layers.get_mut(set).unwrap().switches.solo = true;
        assert!(layers.solo_active());
        assert!(layers.shows(Some(set), false), "soloed");
        assert!(!layers.shows(None, false), "no layer: not soloed");
        assert!(!layers.shows(Some(gone), false));
        assert!(!layers.shows(Some(lights), true));
        assert!(layers.all_animate());
        layers.get_mut(lights).unwrap().switches.animation = false;
        assert!(!layers.all_animate());
    }
}

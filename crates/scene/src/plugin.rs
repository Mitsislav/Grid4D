//! Objects made by plugins (see `docs/reference/extend-plugins.md`): the
//! document keeps only the plugin's type id and the parameter values by name;
//! what they mean is the plugin's (`grid4d-plugin`). A document whose plugin
//! is missing keeps them as they are.

use glam::Vec3;

/// A plugin parameter's value.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum PluginValue {
    Float(f32),
    Int(i32),
    Bool(bool),
    /// An index into the parameter's choices.
    Choice(u32),
    Vector(Vec3),
    /// Linear RGB.
    Color([f32; 3]),
}

impl PluginValue {
    /// Its real numbers (for finiteness checks).
    #[must_use]
    pub fn numbers(&self) -> Vec<f32> {
        match *self {
            Self::Float(v) => vec![v],
            Self::Int(_) | Self::Bool(_) | Self::Choice(_) => Vec::new(),
            Self::Vector(v) => v.to_array().to_vec(),
            Self::Color(c) => c.to_vec(),
        }
    }
}

/// An object a plugin makes: which plugin, and its parameter values by name
/// (in the order they were set).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct PluginObject {
    /// The plugin's namespaced id (`"grid4d.gear"`).
    pub type_id: String,
    pub values: Vec<(String, PluginValue)>,
}

impl PluginObject {
    #[must_use]
    pub fn new(type_id: impl Into<String>) -> Self {
        Self {
            type_id: type_id.into(),
            values: Vec::new(),
        }
    }

    /// The value named `name`, if it has one.
    #[must_use]
    pub fn get(&self, name: &str) -> Option<PluginValue> {
        self.values.iter().find(|(n, _)| n == name).map(|&(_, v)| v)
    }

    /// Sets the value named `name` (added at the end if it had none).
    pub fn set(&mut self, name: &str, value: PluginValue) {
        match self.values.iter_mut().find(|(n, _)| n == name) {
            Some((_, v)) => *v = value,
            None => self.values.push((name.to_owned(), value)),
        }
    }

    /// Every real number of its values.
    #[must_use]
    pub fn numbers(&self) -> Vec<f32> {
        self.values.iter().flat_map(|(_, v)| v.numbers()).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn values_by_name() {
        let mut gear = PluginObject::new("grid4d.gear");
        assert_eq!(gear.get("Teeth"), None);
        gear.set("Teeth", PluginValue::Int(12));
        gear.set("Radius", PluginValue::Float(100.0));
        gear.set("Teeth", PluginValue::Int(20));
        assert_eq!(gear.get("Teeth"), Some(PluginValue::Int(20)));
        assert_eq!(gear.values.len(), 2, "set replaces");
        gear.set("Colour", PluginValue::Color([1.0, f32::NAN, 0.0]));
        assert_eq!(gear.numbers().len(), 4);
    }
}

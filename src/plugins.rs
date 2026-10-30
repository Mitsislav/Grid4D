//! The plugins built into Grid4D. To add one: add its crate to the workspace
//! and to this crate's dependencies, then one line below (see
//! `docs/plugins.md`).

use grid4d_plugin::Registry;

/// Every plugin of this Grid4D.
pub fn registry() -> Registry {
    let mut registry = Registry::default();
    registry.add_generator(grid4d_plugin_gear::Gear);
    registry.add_deformer(grid4d_plugin_wave::Wave);
    registry.add_command(grid4d_plugin_scatter::Scatter);
    registry
}

#[cfg(test)]
mod tests {
    #[test]
    fn the_built_in_plugins_are_listed() {
        let registry = super::registry();
        assert!(registry.generator("grid4d.gear").is_some());
        assert!(registry.deformer("grid4d.wave").is_some());
        assert!(registry.command("grid4d.scatter").is_some());
    }
}

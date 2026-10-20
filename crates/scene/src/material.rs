//! Materials (see `docs/reference/manager-material.md`): surface properties kept
//! once per document in an ordered table and linked to objects by Material tags.

use slotmap::SlotMap;

slotmap::new_key_type! {
    /// Stable, generational handle to a material in [`Materials`]: a removed
    /// material's id never names another one.
    pub struct MaterialId;
}

/// A surface: a small physically based set of properties. Colours are linear RGB.
#[derive(Clone, Debug, PartialEq)]
pub struct Material {
    pub name: String,
    /// The base colour (what the viewport shows).
    pub color: [f32; 3],
    /// 0 mirror-like … 1 matte.
    pub roughness: f32,
    /// 0 dielectric … 1 metal.
    pub metallic: f32,
    /// Light the surface gives off.
    pub emission: [f32; 3],
}

impl Material {
    /// 80 % grey (sRGB), as linear RGB.
    pub const DEFAULT_COLOR: [f32; 3] = [0.604, 0.604, 0.604];

    /// A material with the defaults.
    #[must_use]
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            color: Self::DEFAULT_COLOR,
            roughness: 0.5,
            metallic: 0.0,
            emission: [0.0; 3],
        }
    }

    /// Every number, for finiteness checks.
    #[must_use]
    pub fn numbers(&self) -> [f32; 8] {
        let [r, g, b] = self.color;
        let [er, eg, eb] = self.emission;
        [r, g, b, self.roughness, self.metallic, er, eg, eb]
    }
}

/// The document's materials in Material Manager order.
#[derive(Clone, Debug, Default)]
pub struct Materials {
    table: SlotMap<MaterialId, Material>,
    order: Vec<MaterialId>,
}

impl PartialEq for Materials {
    fn eq(&self, other: &Self) -> bool {
        self.order == other.order && self.order.iter().all(|&id| self.table.get(id) == other.table.get(id))
    }
}

impl Materials {
    #[must_use]
    pub fn len(&self) -> usize {
        self.order.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.order.is_empty()
    }

    #[must_use]
    pub fn get(&self, id: MaterialId) -> Option<&Material> {
        self.table.get(id)
    }

    pub fn get_mut(&mut self, id: MaterialId) -> Option<&mut Material> {
        self.table.get_mut(id)
    }

    #[must_use]
    pub fn contains(&self, id: MaterialId) -> bool {
        self.table.contains_key(id)
    }

    /// Ids in Material Manager order.
    #[must_use]
    pub fn ids(&self) -> &[MaterialId] {
        &self.order
    }

    /// Materials in Material Manager order.
    pub fn iter(&self) -> impl Iterator<Item = (MaterialId, &Material)> + '_ {
        self.order.iter().filter_map(|&id| self.table.get(id).map(|m| (id, m)))
    }

    /// Appends `material`.
    pub fn insert(&mut self, material: Material) -> MaterialId {
        let id = self.table.insert(material);
        self.order.push(id);
        id
    }

    /// Removes the material; `None` if there was none.
    pub fn remove(&mut self, id: MaterialId) -> Option<Material> {
        let material = self.table.remove(id)?;
        self.order.retain(|&m| m != id);
        Some(material)
    }

    /// Position in Material Manager order.
    #[must_use]
    pub fn index_of(&self, id: MaterialId) -> Option<usize> {
        self.order.iter().position(|&m| m == id)
    }

    /// `base` if no material has that name, else `base.1`, `base.2`, … (as for
    /// objects).
    #[must_use]
    pub fn unique_name(&self, base: &str) -> String {
        let taken = |name: &str| self.iter().any(|(_, m)| m.name == name);
        if !taken(base) {
            return base.to_owned();
        }
        (1..)
            .map(|n| format!("{base}.{n}"))
            .find(|name| !taken(name))
            .unwrap_or_else(|| base.to_owned())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ordered_table_with_unique_names_and_stable_ids() {
        let mut materials = Materials::default();
        assert!(materials.is_empty());
        let a = materials.insert(Material::new(materials.unique_name("Mat")));
        let b = materials.insert(Material::new(materials.unique_name("Mat")));
        assert_eq!(materials.get(b).unwrap().name, "Mat.1");
        assert_eq!(materials.ids(), &[a, b]);
        assert_eq!(materials.index_of(b), Some(1));

        let snapshot = materials.clone();
        assert!(materials.remove(a).is_some());
        assert!(!materials.contains(a));
        let c = materials.insert(Material::new("Mat"));
        assert_ne!(c, a, "a removed id never names another material");
        assert_ne!(materials, snapshot);
        assert_eq!(snapshot.ids(), &[a, b], "a copy keeps the ids");
        assert_eq!(
            materials.iter().map(|(_, m)| m.name.as_str()).collect::<Vec<_>>(),
            ["Mat.1", "Mat"]
        );
    }
}

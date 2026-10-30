// A 5 × 5 grid of cubes, coloured from corner to corner.
let group = create("Null");
group.name = "Grid";
for x in 0..5 {
    for z in 0..5 {
        let cube = create("Cube", group);
        cube.name = `Cube ${x}-${z}`;
        cube.position = [(x - 2) * 60, 0, (z - 2) * 60];
        cube.set("Size . X", 40);
        cube.set("Size . Y", 20 + 15 * (x + z));
        cube.set("Size . Z", 40);
        cube.color = [x / 4.0, 0.4, z / 4.0];
    }
}
select(group);
print(`made ${group.children.len()} cubes`);

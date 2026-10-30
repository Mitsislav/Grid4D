// Gives each selected object (or every object, with none selected) a
// display colour of its own. Change the seed for other colours.
let seed = 2026;

// A small random number generator: the next number after `s`.
fn next(s) {
    (s * 1103515245 + 12345) % 2147483648
}

let objects = if selection().is_empty() { objects() } else { selection() };
for object in objects {
    let colour = [];
    for channel in 0..3 {
        seed = next(seed);
        colour.push(0.2 + 0.8 * seed / 2147483648.0);
    }
    object.color = colour;
}
print(`objects coloured: ${objects.len()}`);

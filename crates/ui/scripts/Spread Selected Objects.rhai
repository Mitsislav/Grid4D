// Puts the selected objects in a row along X, 150 cm apart, centred on
// the first one's place, in the order they were selected.
let objects = selection();
if objects.is_empty() {
    throw "select the objects to spread first";
}
let spacing = 150.0;
let start = objects[0].position;
let first = start[0] - spacing * (objects.len() - 1) / 2.0;
for i in 0..objects.len() {
    objects[i].position = [first + spacing * i, start[1], start[2]];
}
print(`objects spread: ${objects.len()}`);

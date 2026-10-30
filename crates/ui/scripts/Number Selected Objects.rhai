// Numbers the selected objects in the order they were selected:
// "Cube" becomes "Cube 1", "Sphere" becomes "Sphere 2", …
let number = 1;
for object in selection() {
    object.name = `${object.name} ${number}`;
    number += 1;
}
print(`objects numbered: ${number - 1}`);

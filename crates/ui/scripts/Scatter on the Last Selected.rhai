// Spreads 30 copies of the selected objects over the surface of the one
// selected last, with the Scatter plugin command, then selects them.
let made = run_command("Scatter", #{
    Count: 30,
    Seed: 7,
    "Scale Variation": 30,
});
select(made);
print("scattered 30 copies");

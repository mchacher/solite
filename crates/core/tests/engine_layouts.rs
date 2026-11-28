//! What one script engine costs (spec 057): QuickJS's own count after each part of the fixed cost,
//! for three layouts of four recipes, and Sowel's seven recipes in one shared context. The figures
//! are printed (`-- --nocapture`); the assertions are that the loads succeed or their reasons are
//! recorded. The measuring code is shared with the QEMU image (`fixtures/engine_study.rs`).

use std::collections::BTreeMap;

use rquickjs::Runtime;

mod study {
    include!("fixtures/engine_study.rs");
}
use study::{Fig, Made};

const SHIM: &str = include_str!("../src/recipes/script/shim.js");
const RECIPES: [(&str, &str); 7] = [
    (
        "motion-light",
        include_str!("../../../recipes/scripts/motion-light.js"),
    ),
    (
        "state-watch",
        include_str!("../../../recipes/scripts/state-watch.js"),
    ),
    (
        "state-trigger-light",
        include_str!("../../../recipes/scripts/state-trigger-light.js"),
    ),
    (
        "schedule-on-off",
        include_str!("../../../recipes/scripts/schedule-on-off.js"),
    ),
    (
        "motion-light-dimmable",
        include_str!("../../../recipes/scripts/motion-light-dimmable.js"),
    ),
    (
        "presence-thermostat",
        include_str!("../../../recipes/scripts/presence-thermostat.js"),
    ),
    (
        "switch-light",
        include_str!("../../../recipes/scripts/switch-light.js"),
    ),
];

/// The stack limit of the engine's runtimes (`SCRIPT_STACK_BYTES`).
const ENGINE_STACK: usize = 16 * 1024;

fn make_with(stack: usize) -> Result<Made, String> {
    let runtime = Runtime::new().map_err(|e| e.to_string())?;
    runtime.set_max_stack_size(stack);
    Ok(Made {
        runtime,
        held: None,
    })
}

/// 32 KB: the engine's 16 KB is too little for the wrapped layout on the Mac (64-bit frames), see
/// `the_shared_layout_at_the_engines_stack_limit`; the figures do not depend on the limit.
fn make() -> Result<Made, String> {
    make_with(2 * ENGINE_STACK)
}

fn print(name: &str, fig: &Fig) {
    println!("engine study: {name} {}", fig.text());
}

#[test]
fn the_parts_of_the_fixed_cost() {
    study::parts(&make, SHIM, RECIPES[0].1, &mut |n, f| print(n, f)).expect("the parts measure");
}

#[test]
fn three_layouts_of_four_recipes() {
    let four = &RECIPES[..4];
    let notes = study::layout_runtimes(&make, SHIM, four, &mut |n, f| print(n, f))
        .expect("one runtime each");
    println!("engine study: one runtime each, notes {notes:?}");
    assert!(notes.is_empty(), "{notes:?}");
    let notes =
        study::layout_contexts(&make, SHIM, four, &mut |n, f| print(n, f)).expect("a context each");
    println!("engine study: a context each, notes {notes:?}");
    assert!(notes.is_empty(), "{notes:?}");
    let shared = study::layout_shared(&make, SHIM, four, &mut |n, f| print(n, f))
        .expect("one shared context");
    for s in &shared {
        println!("engine study: shared {}: {:?}", s.name, s.reason);
        assert!(s.reason.is_none(), "{}: {:?}", s.name, s.reason);
    }
}

#[test]
fn the_seven_recipes_in_one_shared_context() {
    let mut last = Fig::default();
    let shared = study::layout_shared(&make, SHIM, &RECIPES, &mut |n, f| {
        print(n, f);
        last = f.clone();
    })
    .expect("the shared context is made");
    println!("engine study: seven shared {}", last.text());
    let mut table = BTreeMap::new();
    for s in &shared {
        let alone = study::alone_def(
            &make,
            SHIM,
            RECIPES.iter().find(|r| r.0 == s.name).unwrap().1,
        )
        .expect("each recipe loads alone");
        let verdict = match (&s.def, &s.reason) {
            (Some(def), None) if *def == alone => "loads and starts, info unchanged".to_string(),
            (Some(_), None) => "loads and starts, info DIFFERS".to_string(),
            (_, Some(reason)) => reason.clone(),
            (None, None) => "no result".to_string(),
        };
        println!("engine study: shared {}: {verdict}", s.name);
        table.insert(s.name.clone(), verdict);
    }
    assert_eq!(table.len(), RECIPES.len());
}

/// Not an assertion: whether the shared layout loads at the engine's own 16 KB stack limit here.
#[test]
fn the_shared_layout_at_the_engines_stack_limit() {
    let shared = study::layout_shared(
        &|| make_with(ENGINE_STACK),
        SHIM,
        &RECIPES[..4],
        &mut |_, _| {},
    )
    .expect("the shared context is made");
    for s in &shared {
        println!(
            "engine study: shared at 16 KB of stack, {}: {:?}",
            s.name, s.reason
        );
    }
}

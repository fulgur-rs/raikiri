use super::*;
use crate::tests::text_of;

/// Seeds the coverage tests draw documents from.
const SEEDS: std::ops::Range<u64> = 0..400;

#[test]
fn a_seed_gives_the_same_document_every_time() {
    for seed in [0, 1, 2, 77, u64::MAX] {
        assert_eq!(generate(seed), generate(seed), "seed {seed}");
    }
}

#[test]
fn consecutive_seeds_give_different_documents() {
    let documents: std::collections::HashSet<String> = SEEDS.map(generate).collect();
    assert_eq!(documents.len(), SEEDS.count());
}

#[test]
fn a_document_is_a_complete_page_in_the_test_font() {
    for seed in SEEDS {
        let html = generate(seed);
        assert!(
            html.starts_with("<!doctype html>\n<html><head><meta charset=\"utf-8\"><style>\n"),
            "{html}"
        );
        assert!(html.contains("font: 10px/12px Ahem"), "{html}");
        assert!(html.ends_with("</body></html>\n"), "{html}");
    }
}

#[test]
fn the_documents_reach_every_kind_of_box() {
    let all: String = SEEDS.map(generate).collect();
    for expected in [
        "@page { size: ",
        "display:block;",
        "display:flex;",
        "display:grid;",
        "display:inline-block;",
        "display:inline-flex;",
        "display:inline-grid;",
        "display:inline;",
        "<table style=\"",
        "<caption>",
        "<td colspan=2 style=",
        "<td rowspan=2 style=",
        "<td style=",
        "display:table-cell;",
        "column-count:",
        "column-span:all",
        "writing-mode:vertical-rl;",
        "position:absolute;",
        "float:left;",
        "flex-direction:",
        "grid-template-columns:",
        "z-index:",
    ] {
        assert!(all.contains(expected), "{expected:?} never appears");
    }
}

#[test]
fn documents_stay_small() {
    // Tables open their cells whatever is left of the budget, so the bound is
    // looser than the budget.
    for seed in SEEDS {
        let html = generate(seed);
        let boxes = html.matches("style=\"").count();
        assert!(boxes <= 4 * BOX_BUDGET, "seed {seed}: {boxes} boxes");
        assert!(html.len() < 24_000, "seed {seed}: {} bytes", html.len());
    }
}

#[test]
fn every_property_pool_holds_values_that_a_box_can_get() {
    for pool in [BOX, POSITION, ITEM, FLEX, GRID, COLUMNS] {
        for property in pool {
            assert!(!property.values.is_empty(), "{}", property.name);
            assert!((1..=100).contains(&property.percent), "{}", property.name);
            assert!(
                property
                    .name
                    .chars()
                    .all(|c| c.is_ascii_lowercase() || c == '-'),
                "{}",
                property.name
            );
        }
    }
}

#[test]
fn a_property_is_added_with_the_chance_it_names() {
    let mut generator = Generator {
        rng: Rng::new(5),
        budget: BOX_BUDGET,
        out: String::new(),
    };
    let always = [property("width", &["1px"], 100)];
    let never_written = [property("height", &["2px"], 0)];
    assert_eq!(generator.declarations(&always), "width:1px;");
    assert_eq!(generator.declarations(&never_written), "");
    assert_eq!(
        generator.style("flex", &[&always, &never_written]),
        "display:flex;width:1px;"
    );
}

#[test]
fn the_random_numbers_stay_inside_their_bounds() {
    let mut rng = Rng::new(9);
    let mut seen = [false; 5];
    for _ in 0..200 {
        let value = rng.below(5);
        assert!(value < 5);
        seen[value] = true;
    }
    assert!(seen.iter().all(|seen| *seen), "{seen:?}");
    assert!((0..200).any(|_| rng.chance(50)));
    assert!((0..200).any(|_| !rng.chance(50)));
    assert!((0..50).all(|_| rng.chance(100)));
    assert!((0..50).all(|_| !rng.chance(0)));
    assert_eq!(rng.pick(&["only"]), "only");
    assert_ne!(Rng::new(1).next_u64(), Rng::new(2).next_u64());
}

#[test]
fn an_empty_budget_leaves_only_text() {
    let mut generator = Generator {
        rng: Rng::new(3),
        budget: 0,
        out: String::new(),
    };
    generator.element(0);
    assert!(!generator.out.is_empty());
    assert!(!generator.out.contains('<'), "{}", generator.out);
    generator.out.clear();
    generator.budget = BOX_BUDGET;
    generator.element(MAX_DEPTH);
    assert!(!generator.out.contains('<'), "{}", generator.out);
}

#[test]
fn the_documents_lay_out_to_pages_and_boxes() {
    for seed in 0..60 {
        let text = text_of(&generate(seed));
        assert!(text.starts_with("pages: "), "seed {seed}: {text}");
        assert!(text.contains("\nnodes: "), "seed {seed}");
    }
}

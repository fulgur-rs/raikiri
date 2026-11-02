use super::*;
use crate::layout::ifc::test_support::block_fixture;

fn reset() {
    SIBLING_SHIFT_WORK.with(|work| work.set([0; 3]));
}

fn work() -> [usize; 3] {
    SIBLING_SHIFT_WORK.with(|work| work.get())
}

/// The sum of the deltas of the moves that start before `index`, added in
/// the order they were made, by a plain filter over all of them.
fn reference(moves: &[(usize, f32)], index: usize) -> f32 {
    moves
        .iter()
        .filter(|&&(after, _)| after < index)
        .map(|&(_, delta)| delta)
        .sum()
}

/// Record `moves` for the children of a parent that has `children` blocks
/// and check, after every move, that the pending move of every child is the
/// reference sum to the bit. Returns the work of the queries.
fn check_moves(children: usize, moves: &[(usize, f32)]) -> [usize; 3] {
    let mut ids = Vec::new();
    let mut fixture = block_fixture("", |doc, root| {
        for _ in 0..children {
            ids.push(doc.append_element(Some(root), "div", Some("display:block;height:10px")));
        }
    });
    let parent = fixture.root;
    let cascade = crate::layout::test_support::cascaded(&fixture.doc);
    let control = PageLayoutControl::default();
    let mut budget = ProjectionWork::new(&control, 1_000_000);
    let mut shifts = sibling_shifts::SiblingShifts::default();
    let mut recorded = Vec::new();
    let mut queries = [0; 3];
    for &(after, delta) in moves {
        shifts
            .record(&fixture.doc, parent, ids[after], delta, &mut budget)
            .unwrap();
        recorded.push((after, delta));
        let session = crate::layout::context::LayoutSession::bridged(&mut fixture.doc, &cascade);
        reset();
        for (index, &id) in ids.iter().enumerate() {
            // The parent has no moves of its own, so its offset adds nothing.
            let expected = 0.0_f32 + reference(&recorded, index);
            assert_eq!(
                shifts.offset(&session, id).to_bits(),
                expected.to_bits(),
                "child {index} after {} moves",
                recorded.len()
            );
        }
        for (total, count) in queries.iter_mut().zip(work()) {
            *total += count;
        }
    }
    queries
}

#[test]
fn moves_in_order_are_summed_as_the_filter_over_all_of_them_does() {
    // Deltas that do not add exactly in f32, so that another order of
    // addition would show in the bits.
    let moves: Vec<(usize, f32)> = (0..24)
        .map(|k| (k / 2, 0.1_f32 * (k as f32 + 1.0) + 1.0e6 * (k % 3) as f32))
        .collect();
    check_moves(16, &moves);
}

#[test]
fn moves_out_of_order_are_summed_as_the_filter_over_all_of_them_does() {
    // The sequence starts in order, then a move starts before the last one,
    // and later moves go on in order again: the order of the sums stays the
    // order the moves were made.
    let mut moves: Vec<(usize, f32)> = (0..6)
        .map(|k| (k + 4, 0.3_f32 * (k as f32 + 1.0)))
        .collect();
    moves.push((2, 1.0e7 + 0.3));
    moves.extend((0..6).map(|k| (k + 8, 0.7_f32 / (k as f32 + 3.0))));
    moves.push((0, 0.1));
    check_moves(16, &moves);
    // A move that starts at the same child as the one before is in order.
    check_moves(8, &[(3, 1.5), (3, 2.25), (3, 0.1), (5, 7.0), (5, 0.3)]);
}

#[test]
fn the_pending_move_of_a_child_is_read_without_visiting_all_the_moves() {
    for count in [64, 128, 256, 512] {
        let moves: Vec<(usize, f32)> = (0..count - 1).map(|k| (k, 1.0 + k as f32 * 0.25)).collect();
        let [recorded, queries, reads] = check_moves(count, &moves);
        assert_eq!(recorded, 0, "recording is not counted by the queries");
        // A query asks about every child after each of the moves, and every
        // answer reads the moves it needs one by one at most `log2` times.
        let bound = queries * (usize::BITS - count.leading_zeros()) as usize;
        assert!(
            reads <= bound,
            "{count}: {reads} reads for {queries} queries"
        );
        // Reading all the moves for each query would take about this much.
        assert!(reads * 4 < queries * count / 2, "{count}: {reads} reads");
    }
}

/// The top of each of `count` multicol containers that follow one another in
/// a section, on 100px pages. Each holds two 30px blocks in two columns and
/// a 30px spanner that does not break, so a spanner that would cross the end
/// of a page moves to the next one and moves everything after it.
fn spanning_owner_tops(count: usize) -> Vec<f32> {
    paginated_ys(
        |doc, body| {
            let section = doc.append_element(Some(body), "section", Some("display:block"));
            (0..count)
                .map(|_| {
                    let owner = doc.append_element(
                        Some(section),
                        "div",
                        Some("display:block;column-count:2;column-gap:0"),
                    );
                    for _ in 0..2 {
                        doc.append_element(Some(owner), "div", Some("display:block;height:30px"));
                    }
                    doc.append_element(
                        Some(owner),
                        "div",
                        Some("display:block;column-span:all;height:30px;break-inside:avoid"),
                    );
                    owner
                })
                .collect()
        },
        &[],
    )
}

#[test]
fn containers_with_spanners_in_one_parent_ask_for_their_moves_without_a_scan() {
    // Where the containers stood before the moves were read through running
    // sums: a spanner that would cross a page end moves to the next page.
    assert_eq!(
        spanning_owner_tops(16),
        [
            0.0, 60.0, 130.0, 190.0, 250.0, 330.0, 390.0, 450.0, 530.0, 590.0, 650.0, 730.0, 790.0,
            850.0, 930.0, 990.0
        ]
    );
    for count in [32, 64, 128, 256] {
        reset();
        let tops = spanning_owner_tops(count);
        let [recorded, queries, reads] = work();
        // Every three containers fill two 100px pages and a bit.
        for index in 2..count - 3 {
            assert_eq!(tops[index + 3] - tops[index], 200.0, "{count}: {index}");
        }
        // About one move for each container, asked about at every level of
        // the tree for each container.
        assert!(recorded + 2 >= count, "{count}: {recorded} moves");
        assert!(queries >= count, "{count}: {queries} queries");
        // Each query reads the moves about `log2` times, not one by one.
        let bound = queries * (usize::BITS - recorded.leading_zeros() + 1) as usize;
        assert!(
            reads <= bound,
            "{count}: {reads} reads for {queries} queries"
        );
        assert!(
            reads * 8 < recorded * queries,
            "{count}: {reads} reads for {recorded} moves and {queries} queries"
        );
    }
}

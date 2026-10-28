//! Pure logic of holst-core, with allocation counting; no network.

mod fixtures;

use divan::{AllocProfiler, Bencher, black_box};
use holst_core::domain::{
    area::Area,
    board_file::{compression, compression::Compression, document},
    change::value::{BoardChange, ChangeSources, NewEntry, NewObject},
    geometry::{Point, Rectangle, Size},
    id,
    id::BoardId,
    id_source::SequentialIds,
    limit::Limit,
    picture,
    placement,
    time::Timestamp,
    tree,
    tree::{
        name_index::{build::NameIndex, query::NodeFilter},
        walk::TrashWalk,
    },
};

#[global_allocator]
static ALLOC: AllocProfiler = AllocProfiler::system();

const SIZES: &[usize] = &[100, 1_000, 10_000];

fn main() -> holst_core::Result<()> {
    SIZES.iter().try_for_each(|&n| index(n, None).map(drop))?;
    board_doc(PER_FRAME * 2).map(drop)?;
    divan::main();
    Ok(())
}

fn index(n: usize, target: Option<&str>) -> holst_core::Result<NameIndex> {
    NameIndex::from_tree_json(&fixtures::tree(0, n, target).to_string())
}

// --- parsing the tree reply ---------------------------------------------

#[divan::bench(args = SIZES)]
fn tree_value_then_flatten(bencher: Bencher<'_, '_>, n: usize) {
    let raw = fixtures::tree(0, n, None).to_string();
    bencher
        .counter(divan::counter::BytesCount::new(raw.len()))
        .bench(|| {
            serde_json::from_str::<serde_json::Value>(black_box(&raw))
                .map(|value| tree::flatten(&value))
        });
}

#[divan::bench(args = SIZES)]
fn flatten_only(bencher: Bencher<'_, '_>, n: usize) {
    let value = fixtures::tree(0, n, None);
    bencher.bench(|| tree::flatten(black_box(&value)));
}

#[divan::bench(args = SIZES)]
fn parse_tree(bencher: Bencher<'_, '_>, n: usize) {
    let raw = fixtures::tree(0, n, None).to_string();
    bencher
        .counter(divan::counter::BytesCount::new(raw.len()))
        .bench(|| tree::parse(black_box(&raw)).map(|parsed| parsed.nodes));
}

// --- small functions on the hot path ------------------------------------

#[divan::bench]
fn board_id_from_url(bencher: Bencher<'_, '_>) {
    let url = "https://app.holst.so/board/00000001-0000-4000-8000-0000000003e7?x=1";
    bencher.bench(|| BoardId::find_in(black_box(url)).map(String::from));
}

#[divan::bench]
fn board_id_from_name(bencher: Bencher<'_, '_>) {
    bencher
        .bench(|| BoardId::find_in(black_box("Планы на Q4 и ретро")).map(String::from));
}

#[divan::bench]
fn is_uuid(bencher: Bencher<'_, '_>) {
    let samples = [
        "ae1f0a2b-c3d4-4f19-8a7b-1c2d3e4f5a6b",
        "ae1f0a2bc3d44f198a7b1c2d3e4f5a6babcd",
        "Планы на Q4",
    ];
    bencher.bench(|| black_box(samples).iter().filter(|s| id::is_uuid(s)).count());
}

#[divan::bench]
fn uuid_spans(bencher: Bencher<'_, '_>) {
    let text = "доска «Планы» не найдена в easy (00000000-0000-4000-8000-000000000001). \
                Похожие: 00000001-0000-4000-8000-00000000000a, \
                00000001-0000-4000-8000-000000001388; корзина.";
    bencher
        .counter(divan::counter::BytesCount::of_str(text))
        .bench(|| id::uuid_spans(black_box(text)).count());
}

#[divan::bench]
fn workspaces_from_current_user(bencher: Bencher<'_, '_>) {
    let me = fixtures::current_user(20);
    bencher.bench(|| holst_core::domain::account::account_workspaces(black_box(&me)));
}

// --- the compact search index -------------------------------------------

#[divan::bench(args = SIZES)]
fn index_from_json(bencher: Bencher<'_, '_>, n: usize) {
    let raw = fixtures::tree(0, n, None).to_string();
    bencher
        .counter(divan::counter::BytesCount::new(raw.len()))
        .bench(|| NameIndex::from_tree_json(black_box(&raw)));
}

#[divan::bench(args = SIZES)]
fn index_search_top10(bencher: Bencher<'_, '_>, n: usize) {
    let index = index(n, Some("Целевая доска")).unwrap_or_default();
    bencher.bench(|| {
        index.search(black_box("целевая"), NodeFilter::default(), Limit::from(10))
    });
}

#[divan::bench(args = SIZES)]
fn index_walk(bencher: Bencher<'_, '_>, n: usize) {
    let index = index(n, None).unwrap_or_default();
    bencher.bench(|| index.walk(None, black_box(TrashWalk::Hidden)));
}

#[divan::bench(args = SIZES)]
fn index_search_words(bencher: Bencher<'_, '_>, n: usize) {
    let index = index(n, None).unwrap_or_default();
    bencher.bench(|| {
        index.search(
            black_box("планирование доска 0-99"),
            NodeFilter::default(),
            Limit::from(10),
        )
    });
}

// --- a big board: load, list, draw, save ---------------------------------

const BOARD_SIZES: &[usize] = &[200, 4_000];

const PER_FRAME: usize = 40;

fn board_doc(n: usize) -> holst_core::Result<yrs::Doc> {
    let doc = yrs::Doc::with_client_id(1);
    let ids = SequentialIds::new("00000000-0000-4000-8000");
    let fresh = ChangeSources {
        built_at: Timestamp::from_millis(1_700_000_000_000),
        ids: &ids,
    };
    let user = fixtures::uuid(2, 0);
    let grid = |i: usize, columns: usize, step: f64| {
        Point::new(
            f64::from(u32::try_from(i % columns).unwrap_or(0)) * step,
            f64::from(u32::try_from(i / columns).unwrap_or(0)) * step,
        )
    };
    (0..n.div_ceil(PER_FRAME)).try_for_each(|f| {
        let frame = BoardChange::AddAll(vec![NewEntry {
            new: NewObject::Frame {
                title: format!("Раздел {f}"),
                at: grid(f, 10, 2000.0)?,
                size: Some(Size::new(1800.0, 1800.0)?),
                color: None,
            },
            frame: None,
        }]);
        let built =
            holst_core::domain::change::build::build(&doc, &frame, &user, &fresh)?;
        let id = built
            .objects
            .first()
            .map(|t| t.id.clone())
            .unwrap_or_default();
        (0..PER_FRAME.min(n - f * PER_FRAME)).try_for_each(|s| {
            let sticker = NewObject::Sticker {
                text: format!("Идея {f}-{s}: проверить гипотезу\nи записать вывод"),
                at: grid(s, 8, 210.0)?,
                size: None,
                color: None,
            };
            let op = BoardChange::AddAll(vec![NewEntry {
                new: sticker,
                frame: Some(id.clone()),
            }]);
            holst_core::domain::change::build::build(&doc, &op, &user, &fresh).map(drop)
        })
    })?;
    Ok(doc)
}

fn board_state(n: usize) -> Vec<u8> {
    board_doc(n)
        .map(|doc| document::encoded(&doc))
        .unwrap_or_default()
}

#[divan::bench(args = BOARD_SIZES)]
fn board_restore(bencher: Bencher<'_, '_>, n: usize) {
    let saved = board_state(n);
    bencher
        .counter(divan::counter::BytesCount::new(saved.len()))
        .bench(|| document::decoded(black_box(&saved)).map(drop));
}

#[divan::bench(args = BOARD_SIZES)]
fn board_objects(bencher: Bencher<'_, '_>, n: usize) {
    let doc = document::decoded(&board_state(n)).ok();
    bencher.bench(|| {
        doc.as_ref()
            .map(|doc| holst_core::domain::board::objects(black_box(doc)).len())
    });
}

#[divan::bench(args = BOARD_SIZES)]
fn board_snapshot_frame(bencher: Bencher<'_, '_>, n: usize) {
    let objects = document::decoded(&board_state(n))
        .map(|doc| holst_core::domain::board::objects(&doc))
        .unwrap_or_default();
    let inlined = std::collections::HashMap::new();
    let assets = picture::PictureImages {
        storage: "https://s/",
        replacements: &inlined,
    };
    let request = picture::PictureRequest {
        area: Area::Frame("Раздел 1".into()),
        padding: None,
        scale: picture::PixelsPerUnit::ONE,
        max_side: picture::MAX_SIDE_FILE,
    };
    bencher.bench(|| {
        picture::draw_with(black_box(&objects), &request, &assets)
            .map(|drawn| drawn.picture.svg.len())
    });
}

#[divan::bench(args = BOARD_SIZES)]
fn board_pack(bencher: Bencher<'_, '_>, n: usize) {
    let saved = board_state(n);
    bencher
        .counter(divan::counter::BytesCount::new(saved.len()))
        .bench(|| {
            compression::compressed(black_box(&saved), Compression::Gzip)
                .map(|gz| gz.len())
        });
}

#[divan::bench(args = BOARD_SIZES)]
fn board_unpack(bencher: Bencher<'_, '_>, n: usize) {
    let saved = board_state(n);
    let gz = compression::compressed(&saved, Compression::Gzip)
        .map(std::borrow::Cow::into_owned)
        .unwrap_or_default();
    bencher
        .counter(divan::counter::BytesCount::new(saved.len()))
        .bench(|| compression::uncompressed(black_box(&gz)).map(|state| state.len()));
}

#[divan::bench(args = BOARD_SIZES)]
fn board_import_plan(bencher: Bencher<'_, '_>, n: usize) {
    let saved = board_state(n);
    bencher
        .counter(divan::counter::BytesCount::new(saved.len()))
        .bench(|| {
            document::saved_objects(black_box(&saved))
                .and_then(|count| {
                    document::import_plan(
                        &yrs::Doc::new(),
                        &saved,
                        count,
                        document::Overwrite::Merge,
                    )
                })
                .map(|plan| plan.after)
        });
}

#[divan::bench(args = BOARD_SIZES)]
fn board_draw_frame(bencher: Bencher<'_, '_>, n: usize) {
    let objects = document::decoded(&board_state(n))
        .map(|doc| holst_core::domain::board::objects(&doc))
        .unwrap_or_default();
    let request = picture::PictureRequest {
        area: Area::Frame("Раздел 1".into()),
        padding: None,
        scale: picture::PixelsPerUnit::ONE,
        max_side: picture::MAX_SIDE_FILE,
    };
    bencher.bench(|| {
        picture::draw(black_box(&objects), &request, "https://s/")
            .map(|linked| linked.picture.svg.len())
    });
}

// --- placing a new object in a full frame ---------------------------------

const PLACEMENT_SIZES: &[usize] = &[100, 300, 1_000];

const PLACEMENT_COLUMNS: usize = 25;

fn packed_frame(n: usize) -> (Rectangle, Vec<Rectangle>) {
    let cell = |count: usize| f64::from(u32::try_from(count).unwrap_or(0));
    let rows = n.div_ceil(PLACEMENT_COLUMNS);
    let frame = Rectangle::new(
        0.0,
        0.0,
        cell(PLACEMENT_COLUMNS) * 216.0 + 24.0,
        cell(rows) * 216.0 + 24.0,
    );
    let taken = (0..n)
        .map(|i| {
            Rectangle::new(
                cell(i % PLACEMENT_COLUMNS) * 216.0 + 24.0,
                cell(i / PLACEMENT_COLUMNS) * 216.0 + 24.0,
                192.0,
                192.0,
            )
        })
        .collect();
    (frame, taken)
}

#[divan::bench(args = PLACEMENT_SIZES, sample_count = 3, sample_size = 1)]
fn free_spot_in_full_frame(bencher: Bencher<'_, '_>, n: usize) {
    let (frame, taken) = packed_frame(n);
    bencher.bench(|| placement::free_corner(frame, black_box(&taken), Size::STICKER));
}

#[divan::bench(args = BOARD_SIZES)]
fn board_draw_whole(bencher: Bencher<'_, '_>, n: usize) {
    let objects = document::decoded(&board_state(n))
        .map(|doc| holst_core::domain::board::objects(&doc))
        .unwrap_or_default();
    let request = picture::PictureRequest {
        area: Area::Board,
        padding: None,
        scale: picture::PixelsPerUnit::ONE,
        max_side: picture::MAX_SIDE_FILE,
    };
    bencher.bench(|| {
        picture::draw(black_box(&objects), &request, "https://s/")
            .map(|linked| linked.picture.svg.len())
    });
}

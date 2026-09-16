//! The wisp, rasterised — the same six moods the core keeps as SVG, drawn
//! this time into pixels. Keeping both means a change to the wisp has to come
//! out the same through either surface.
//!
//! `GLIMMERWOOD_BLESS=1` writes the pictures again.

use glimmerwood_core::dose::{Mode, Trend};
use glimmerwood_core::wisp::Wisp;
use glimmerwood_raster::Raster;

const NOOK: (u32, u32) = (152, 56);
/// Long enough for everything that eases to have arrived.
const FRAMES: usize = 400;

fn render(dose: f64, mode: Mode, trend: Trend, night: bool, dark: bool) -> Vec<u8> {
    let mut wisp = Wisp::new(f64::from(NOOK.0), f64::from(NOOK.1));
    wisp.update(dose, mode, trend, false, night, false);
    let mut last = Vec::new();
    for frame in 0..FRAMES {
        let mut raster = Raster::new(NOOK.0, NOOK.1).expect("a nook-sized pixmap");
        wisp.draw(frame as f64 * 16.0, true, dark, &mut raster);
        last = raster.png().expect("the pixels encode as a PNG");
    }
    last
}

#[test]
fn the_wisp_rasterises_the_same_way_everywhere() {
    let moods: [(&str, f64, Mode, Trend, bool, bool); 6] = [
        (
            "rested",
            0.05,
            Mode::Nourishing,
            Trend::Falling,
            false,
            false,
        ),
        ("everyday", 0.35, Mode::Holding, Trend::Steady, false, false),
        ("clouded", 0.62, Mode::Draining, Trend::Rising, false, false),
        ("drained", 0.92, Mode::Draining, Trend::Rising, false, false),
        ("night", 0.4, Mode::Resting, Trend::Falling, true, false),
        ("away-dark", 0.3, Mode::Away, Trend::Steady, false, true),
    ];

    let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/wisp/");
    let bless = std::env::var_os("GLIMMERWOOD_BLESS").is_some();
    for (name, dose, mode, trend, night, dark) in moods {
        let drawn = render(dose, mode, trend, night, dark);
        let path = format!("{dir}{name}.png");
        if bless {
            std::fs::write(&path, &drawn).expect("write the wisp's picture");
            continue;
        }
        let kept = std::fs::read(&path)
            .unwrap_or_else(|_| panic!("{name}.png is missing; run with GLIMMERWOOD_BLESS=1"));
        assert!(
            kept == drawn,
            "the wisp rasterises {name} differently now; look at it, and if it \
             is right run with GLIMMERWOOD_BLESS=1"
        );
    }
}

#[test]
fn a_nook_with_no_room_in_it_is_refused_rather_than_drawn() {
    assert!(Raster::new(0, 0).is_none());
}

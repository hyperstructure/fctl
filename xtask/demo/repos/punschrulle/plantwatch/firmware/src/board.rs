//! Which board a build is for: `BOARD=kitchen cargo run --release`.

pub struct Board {
    pub id: &'static str,
    pub pot: &'static str,
}

const BOARDS: &[Board] = &[
    Board { id: "living-room", pot: "Monstera" },
    Board { id: "kitchen", pot: "Basil and thyme" },
    Board { id: "bedroom", pot: "Snake plant" },
    Board { id: "balcony", pot: "Tomatoes" },
];

/// The board named at build time.
pub fn this() -> &'static Board {
    let id = env!("BOARD");
    BOARDS
        .iter()
        .find(|b| b.id == id)
        .unwrap_or(&BOARDS[0])
}

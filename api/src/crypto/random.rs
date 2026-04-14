use rand::TryRng;

pub fn get_rng() -> impl TryRng {
    rand::rng()
}

use rand::RngCore;

pub fn get_rng() -> impl RngCore {
    rand::thread_rng()
}

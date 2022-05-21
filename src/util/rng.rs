use std::cell::RefCell;

use rand::{distributions::Alphanumeric, rngs::SmallRng, Rng, SeedableRng};

thread_local! {
  static RNG: RefCell<SmallRng> = RefCell::new(SmallRng::from_entropy());
}

pub fn random_id() -> String {
  RNG.with(|rng| {
    let mut rng = rng.borrow_mut();
    std::iter::repeat(())
      .map(|()| rng.sample(Alphanumeric))
      .map(char::from)
      .take(6)
      .collect()
  })
}

#[inline]
pub fn random_number(end: usize) -> usize {
  RNG.with(|rng| rng.borrow_mut().gen_range(0..end))
}

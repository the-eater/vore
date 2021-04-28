use crate::daemon::Daemon;
use vore_core::init_logging;

mod daemon;

fn main() {
    init_logging();

    let mut daemon = Daemon::new().unwrap();
    daemon.run().unwrap();
}

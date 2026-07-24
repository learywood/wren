use std::{
    env, thread,
    time::{Duration, Instant},
};

use tracy_client::Client;

const CAPTURE_ENVIRONMENT_VARIABLE: &str = "WREN_TRACY_CAPTURE";
const CONNECTION_TIMEOUT: Duration = Duration::from_secs(10);
const POLL_INTERVAL: Duration = Duration::from_millis(1);

pub struct Session {
    capture_required: bool,
    _client: Client,
}

impl Session {
    pub fn start() -> Result<Self, &'static str> {
        let client = Client::start();
        let capture_required = env::var_os(CAPTURE_ENVIRONMENT_VARIABLE).is_some();

        if capture_required {
            wait_until(Client::is_connected)
                .map_err(|()| "Tracy did not connect within 10 seconds")?;
        }

        Ok(Self {
            capture_required,
            _client: client,
        })
    }

    pub fn finish(self) -> Result<(), &'static str> {
        if self.capture_required {
            wait_until(|| !Client::is_connected())
                .map_err(|()| "Tracy did not finish capture within 10 seconds")?;
        }
        Ok(())
    }
}

fn wait_until(condition: impl Fn() -> bool) -> Result<(), ()> {
    let deadline = Instant::now() + CONNECTION_TIMEOUT;
    while !condition() {
        if Instant::now() >= deadline {
            return Err(());
        }
        thread::sleep(POLL_INTERVAL);
    }
    Ok(())
}

//! The HTTPS client behind [`Fetch`]: ureq 3.5 on rustls with bundled webpki roots.
//!
//! The agent refuses plain http, on the first request and on every redirect hop, and follows
//! at most five redirects (GitHub release downloads redirect once to their asset host). The
//! body is read unbounded here; `fetch_capped` caps it at the pinned length and hashes it.

use std::io;
use std::time::Duration;

use ureq::ResponseExt as _;

use super::{Fetch, FetchResponse};

/// Redirects a release download may take before it is refused.
pub const MAX_REDIRECTS: u32 = 5;

pub struct HttpsFetcher {
    agent: ureq::Agent,
}

impl Default for HttpsFetcher {
    fn default() -> Self {
        Self::new()
    }
}

impl HttpsFetcher {
    pub fn new() -> Self {
        let config = ureq::Agent::config_builder()
            .https_only(true)
            .max_redirects(MAX_REDIRECTS)
            .timeout_global(Some(Duration::from_secs(300)))
            .user_agent("linkboy")
            .build();
        Self {
            agent: config.into(),
        }
    }

    pub fn agent(&self) -> &ureq::Agent {
        &self.agent
    }
}

impl Fetch for HttpsFetcher {
    fn get(&self, url: &str) -> Result<FetchResponse, io::Error> {
        if !url.starts_with("https://") {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("{url} is not https"),
            ));
        }
        let response = self.agent.get(url).call().map_err(io::Error::other)?;
        let final_url = response.get_uri().to_string();
        let content_length = response
            .headers()
            .get("content-length")
            .and_then(|value| value.to_str().ok()?.parse().ok());
        Ok(FetchResponse {
            final_url,
            content_length,
            body: Box::new(response.into_body().into_reader()),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_agent_refuses_http_and_bounds_redirects() {
        let fetcher = HttpsFetcher::new();
        let config = fetcher.agent().config();
        assert!(config.https_only());
        assert_eq!(config.max_redirects(), MAX_REDIRECTS);
    }

    #[test]
    fn a_plain_http_url_is_refused_before_any_request() {
        let error = HttpsFetcher::new()
            .get("http://release.example/firmware.uf2")
            .err()
            .expect("http is refused");
        assert_eq!(error.kind(), io::ErrorKind::InvalidInput);
    }

    #[test]
    fn ureq_itself_refuses_http_when_asked_directly() {
        // https_only is checked before any connection is opened.
        let error = HttpsFetcher::new()
            .agent()
            .get("http://127.0.0.1:9/never-contacted")
            .call()
            .unwrap_err();
        assert!(
            matches!(error, ureq::Error::RequireHttpsOnly(_)),
            "{error:?}"
        );
    }
}

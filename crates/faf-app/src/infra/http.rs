//! Shared HTTP transport policy for infrastructure adapters.
//!
//! `reqwest::Client` is deliberately cheap to clone: clones retain the same
//! connection pool and TLS state. Keeping one process-wide instance avoids a
//! separate idle pool for every FAF capability while the capability-specific
//! port traits remain small and independently testable.

use std::{sync::OnceLock, time::Duration};

static HTTP: OnceLock<reqwest::Client> = OnceLock::new();

pub(crate) fn shared_http_client() -> reqwest::Client {
    HTTP.get_or_init(|| {
        client_builder()
            // Do not set a whole-request timeout here: the same transport is
            // used for large map/mod downloads and uploads. Individual API
            // operations can impose a tighter timeout when appropriate.
            .build()
            .expect("the shared HTTP client configuration is valid")
    })
    .clone()
}

/// A separate client for downloads whose redirects must be inspected one hop
/// at a time. Using `Policy::none` lets the owning adapter reject an untrusted
/// destination before any request reaches it.
pub(crate) fn no_redirect_http_client() -> reqwest::Client {
    client_builder()
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .expect("the no-redirect HTTP client configuration is valid")
}

/// A client for the two downloads whose bytes end up being executed: the client
/// installer and the map generator's JAR.
///
/// FAF decided against signing these, on the grounds that fetching them over
/// HTTPS from GitHub is enough: a man in the middle would need a valid
/// certificate for github.com, and anybody who has that can already run code on
/// the machine without bothering to fake a map generator. That reasoning is
/// sound, and it rests entirely on the connection actually staying HTTPS.
///
/// It did not. Both downloads start at a pinned `https://github.com/...` URL
/// and are then followed through redirects by a client using reqwest's default
/// policy, which does not refuse a downgrade. GitHub redirects release assets
/// to its own CDN, so the hop is expected and has to be allowed; what must not
/// be allowed is a hop to plain HTTP, where there is no certificate to present
/// and the argument collapses.
///
/// This policy follows redirects, and refuses any hop that is not HTTPS before
/// the request is sent rather than after the bytes have arrived.
pub(crate) fn https_only_download_client() -> reqwest::Client {
    client_builder()
        .redirect(reqwest::redirect::Policy::custom(
            |attempt| match redirect_verdict(attempt.url().scheme(), attempt.previous().len()) {
                RedirectVerdict::Follow => attempt.follow(),
                RedirectVerdict::TooManyHops => attempt.stop(),
                RedirectVerdict::LeavesHttps => {
                    let refused = HttpsDowngrade(attempt.url().to_string());
                    attempt.error(refused)
                }
            },
        ))
        .build()
        .expect("the HTTPS-only download client configuration is valid")
}

#[derive(Debug, PartialEq, Eq)]
enum RedirectVerdict {
    Follow,
    TooManyHops,
    LeavesHttps,
}

/// The decision itself, apart from reqwest's types.
///
/// `reqwest::redirect::Attempt` has no public constructor, so a test cannot
/// build one to hand to the policy. Taking the two things the decision actually
/// depends on makes the rule testable without pretending to be reqwest.
fn redirect_verdict(scheme: &str, hops: usize) -> RedirectVerdict {
    if scheme != "https" {
        return RedirectVerdict::LeavesHttps;
    }
    // The same ceiling reqwest's default policy uses.
    if hops >= 10 {
        return RedirectVerdict::TooManyHops;
    }
    RedirectVerdict::Follow
}

#[derive(Debug)]
struct HttpsDowngrade(String);

impl std::fmt::Display for HttpsDowngrade {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "refusing a redirect that leaves HTTPS, to {}", self.0)
    }
}

impl std::error::Error for HttpsDowngrade {}

fn client_builder() -> reqwest::ClientBuilder {
    reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(15))
        // FAF's own services see this, so it is the most visible place the old
        // name was still showing. A User-Agent product token cannot contain a
        // space, hence the slug rather than the display name.
        .user_agent(concat!("FAForeverClient/", env!("CARGO_PKG_VERSION")))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// This policy is the whole of the mitigation FAF chose over signing the
    /// map generator and the installer, so it is asserted rather than assumed.
    #[test]
    fn a_redirect_may_not_leave_https() {
        // GitHub redirects a release asset to its own CDN, so an https hop has
        // to keep working.
        assert_eq!(redirect_verdict("https", 0), RedirectVerdict::Follow);
        assert_eq!(redirect_verdict("https", 3), RedirectVerdict::Follow);

        // The case the default policy allows and this one does not. Without a
        // certificate to present there is nothing left of the argument that
        // fetching over HTTPS from GitHub is enough.
        assert_eq!(redirect_verdict("http", 0), RedirectVerdict::LeavesHttps);
        assert_eq!(redirect_verdict("ftp", 0), RedirectVerdict::LeavesHttps);

        // A redirect loop is still a redirect loop.
        assert_eq!(redirect_verdict("https", 10), RedirectVerdict::TooManyHops);
    }

    /// A downgrade is refused before the request goes out, and says so.
    #[test]
    fn the_refusal_names_the_url_it_refused() {
        let refused = HttpsDowngrade("http://elsewhere.invalid/a.jar".to_string());
        assert!(refused
            .to_string()
            .contains("http://elsewhere.invalid/a.jar"));
        assert!(refused.to_string().contains("leaves HTTPS"));
    }
}

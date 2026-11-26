//! [`UrlTrust`]: how much the app trusts a page that it loads from a URL.

#[cfg(any(target_arch = "wasm32", test))]
use crate::{Error, Url};

/// The `sandbox` attribute of the `<iframe>` of an isolated page.
///
/// Without `allow-same-origin`, the page gets an opaque origin. Without the other flags, it
/// can't navigate the app's window, show dialogs, or download files, and the windows it opens
/// are in the same sandbox.
#[cfg(any(target_arch = "wasm32", test))]
pub(crate) const ISOLATED_SANDBOX: &str = "allow-scripts allow-forms allow-popups";

/// [`ISOLATED_SANDBOX`], for a page that keeps the origin of its URL.
#[cfg(any(target_arch = "wasm32", test))]
const OWN_ORIGIN_SANDBOX: &str = "allow-scripts allow-forms allow-popups allow-same-origin";

/// How much the app trusts a page that it loads from a URL.
///
/// Only the web backend uses this. There, a page is an `<iframe>` in the app's own document,
/// and a page that the browser doesn't isolate can read the app's data and act as the app.
/// Natively, the engine keeps every page apart from the app, whatever the trust.
///
/// The trust also applies to every page that the page navigates to.
///
/// A page given as [`crate::WebViewSource::Html`] is always isolated, like
/// [`Self::Isolated`].
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum UrlTrust {
    /// For a URL from anywhere, such as one that a user typed or that came with a file.
    ///
    /// The page runs in a sandbox, with an origin that it shares with no other page. To the
    /// app it is a cross-origin page, even if its URL has the app's origin.
    ///
    /// The page has no cookies and no storage, so a user can't sign in to it. It can't
    /// navigate the app's window, show dialogs, or download files.
    ///
    /// Only `http` and `https` URLs can be loaded.
    #[default]
    Isolated,

    /// For the URL of another site that should work as it does in a browser tab.
    ///
    /// The page runs in the sandbox of [`Self::Isolated`], but keeps the origin of its URL.
    /// So it has its cookies and storage, and a user can sign in to it.
    ///
    /// A URL with the app's own origin is refused, because its page could remove the sandbox
    /// and then control the app. A page that redirects or navigates to the app's origin
    /// isn't refused. So don't use this if anyone else can put a page on the app's origin.
    ///
    /// Only `http` and `https` URLs can be loaded.
    OwnOrigin,

    /// For a URL that the app chose itself, such as that of its own documentation.
    ///
    /// The page runs without a sandbox. If it has the app's origin, it can read and change
    /// everything the app can, and the app can run scripts in it. Any URL can be loaded.
    Full,
}

#[cfg(any(target_arch = "wasm32", test))]
impl UrlTrust {
    /// The `sandbox` attribute of the `<iframe>` of a page with this trust, or `None` for an
    /// `<iframe>` without a sandbox.
    pub(crate) fn sandbox(self) -> Option<&'static str> {
        match self {
            Self::Isolated => Some(ISOLATED_SANDBOX),
            Self::OwnOrigin => Some(OWN_ORIGIN_SANDBOX),
            Self::Full => None,
        }
    }

    /// Checks that an app may load `url` with this trust.
    ///
    /// `app_origin` is the origin of the app's document, as `window.location.origin`
    /// serializes it.
    ///
    /// # Errors
    /// [`Error::UnsupportedScheme`] and [`Error::AppOrigin`]. See the variants of [`Self`].
    pub(crate) fn check(self, app_origin: &str, url: &Url) -> Result<(), Error> {
        let refuses_app_origin = match self {
            Self::Isolated => false,
            Self::OwnOrigin => true,
            Self::Full => return Ok(()),
        };
        // In an `<iframe>` that keeps its origin, a `javascript:` URL runs in a document with
        // the app's origin, and so does a `blob:` URL that the app created. `Isolated` refuses
        // them too, so that both kinds of sandboxed page load the same URLs.
        if !matches!(url.scheme(), "http" | "https") {
            return Err(Error::UnsupportedScheme { url: url.clone() });
        }
        if refuses_app_origin && url.origin().ascii_serialization() == app_origin {
            return Err(Error::AppOrigin { url: url.clone() });
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const APP_ORIGIN: &str = "https://app.example";

    fn check(trust: UrlTrust, url: &str) -> Result<(), Error> {
        let url = Url::parse(url).unwrap_or_else(|err| panic!("Invalid URL {url:?}: {err}"));
        trust.check(APP_ORIGIN, &url)
    }

    #[test]
    fn only_full_trust_has_no_sandbox() {
        for trust in [UrlTrust::Isolated, UrlTrust::OwnOrigin] {
            let sandbox = trust.sandbox().unwrap_or_default();
            assert!(
                sandbox.split(' ').any(|flag| flag == "allow-scripts"),
                "{trust:?} pages must run scripts, got {sandbox:?}"
            );
            assert!(
                !sandbox.contains("allow-top-navigation"),
                "{trust:?} pages must not navigate the app's window, got {sandbox:?}"
            );
        }
        assert_eq!(UrlTrust::Full.sandbox(), None);
    }

    #[test]
    fn only_own_origin_keeps_the_origin_in_the_sandbox() {
        let keeps_origin = |trust: UrlTrust| {
            trust
                .sandbox()
                .is_some_and(|sandbox| sandbox.split(' ').any(|flag| flag == "allow-same-origin"))
        };
        assert!(!keeps_origin(UrlTrust::Isolated));
        assert!(keeps_origin(UrlTrust::OwnOrigin));
    }

    #[test]
    fn sandboxed_pages_only_load_http_and_https() {
        for trust in [UrlTrust::Isolated, UrlTrust::OwnOrigin] {
            for url in ["https://other.example/", "http://other.example/"] {
                assert!(check(trust, url).is_ok(), "{trust:?} must load {url:?}");
            }
            for url in [
                "javascript:alert(1)",
                "blob:https://app.example/1234",
                "data:text/html,<p>hi",
                "about:blank",
                "file:///etc/passwd",
            ] {
                assert!(
                    matches!(check(trust, url), Err(Error::UnsupportedScheme { .. })),
                    "{trust:?} must refuse {url:?}"
                );
            }
        }
    }

    #[test]
    fn own_origin_refuses_the_origin_of_the_app() {
        for url in [
            "https://app.example/page",
            "https://app.example:443/page",
            "https://APP.example/",
        ] {
            assert!(
                matches!(
                    check(UrlTrust::OwnOrigin, url),
                    Err(Error::AppOrigin { .. })
                ),
                "{url:?} has the app's origin"
            );
        }
        for url in [
            "http://app.example/",
            "https://app.example:8443/",
            "https://sub.app.example/",
        ] {
            assert!(
                check(UrlTrust::OwnOrigin, url).is_ok(),
                "{url:?} has another origin than the app"
            );
        }
    }

    #[test]
    fn isolated_and_full_load_the_origin_of_the_app() {
        for trust in [UrlTrust::Isolated, UrlTrust::Full] {
            assert!(check(trust, "https://app.example/page").is_ok());
        }
        assert!(check(UrlTrust::Full, "blob:https://app.example/1234").is_ok());
    }
}

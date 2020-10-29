#[cfg(feature = "cookies")]
use std::sync::RwLock;

#[cfg(feature = "cookies")]
use cookie_store::CookieStore;
#[cfg(feature = "cookies")]
use url::Url;

#[cfg(feature = "cookies")]
#[derive(Debug)]
pub(crate) struct CookieTin {
    inner: RwLock<CookieStore>,
}

#[cfg(feature = "cookies")]
impl CookieTin {
    pub(crate) fn new(store: CookieStore) -> Self {
        CookieTin {
            inner: RwLock::new(store),
        }
    }
    pub(crate) fn get_request_cookies(&self, url: &Url) -> Vec<cookie::Cookie> {
        let store = self.inner.read().unwrap();
        store
            .get_request_cookies(url)
            .map(|c| cookie::Cookie::new(c.name().to_owned(), c.value().to_owned()))
            .collect()
    }

    pub(crate) fn store_response_cookies<I>(&self, cookies: I, url: &Url)
    where
        I: Iterator<Item = cookie::Cookie<'static>>,
    {
        let mut store = self.inner.write().unwrap();
        store.store_response_cookies(cookies, url)
    }
}

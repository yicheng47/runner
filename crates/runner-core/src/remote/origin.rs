use url::Url;

pub fn canonical_origin(value: &str) -> Result<String, &'static str> {
    let url = Url::parse(value).map_err(|_| "Enter a valid service origin.")?;
    if url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.path() != "/"
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return Err("Service origin must have no path, user info, query or fragment.");
    }
    if url.scheme() != "https"
        && !(url.scheme() == "http" && matches!(url.host_str(), Some("localhost" | "127.0.0.1")))
    {
        return Err("Service origin must use HTTPS; explicit HTTP localhost is allowed.");
    }
    Ok(url.as_str().trim_end_matches('/').to_owned())
}
pub fn same_origin(origin: &str, link: &str) -> bool {
    match (Url::parse(origin), Url::parse(link)) {
        (Ok(origin), Ok(link)) => {
            origin.origin() == link.origin()
                && link.username().is_empty()
                && link.password().is_none()
        }
        _ => false,
    }
}
pub fn valid_relay_url(origin: &str, relay: &str) -> bool {
    let (Ok(origin), Ok(relay)) = (Url::parse(origin), Url::parse(relay)) else {
        return false;
    };
    relay.scheme()
        == if origin.scheme() == "https" {
            "wss"
        } else {
            "ws"
        }
        && relay.host_str() == origin.host_str()
        && relay.port_or_known_default() == origin.port_or_known_default()
        && relay.username().is_empty()
        && relay.password().is_none()
        && relay.path() == "/relay/host"
        && relay.query().is_none()
        && relay.fragment().is_none()
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn origins_and_links() {
        assert_eq!(
            canonical_origin("https://Runner.Example:443/").unwrap(),
            "https://runner.example"
        );
        for bad in [
            "http://example.com",
            "http://[::1]",
            "ftp://localhost",
            "https://u:p@example.com",
            "https://example.com/path",
            "https://example.com?q=1",
            "https://example.com/#x",
        ] {
            assert!(canonical_origin(bad).is_err(), "{bad}");
        }
        for good in [
            "https://example.com",
            "http://localhost:8000",
            "http://127.0.0.1:8000",
        ] {
            assert!(canonical_origin(good).is_ok());
        }
        assert!(same_origin(
            "https://example.com",
            "https://example.com/hosts/add?code=BCDF-GHJK"
        ));
        assert!(!same_origin(
            "https://example.com",
            "https://other.com/hosts/add"
        ));
        assert!(valid_relay_url(
            "https://example.com",
            "wss://example.com/relay/host"
        ));
        assert!(!valid_relay_url(
            "https://example.com",
            "ws://example.com/relay/host"
        ));
    }
}

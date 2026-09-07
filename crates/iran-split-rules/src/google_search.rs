/// Registrable names Google Search and Gemini-in-Search load besides
/// `google.com`. A pin on `google.com` does not cover these, so the search
/// page can leave through Windscribe while scripts and APIs stay on MATCH
/// (Hiddify) and the UI stays the restricted “basic” Google.
pub const GOOGLE_SEARCH_COMPANION_DOMAINS: &[&str] = &[
    "gstatic.com",
    "googleapis.com",
    "googleusercontent.com",
    "googletagmanager.com",
];

/// True when pinning this exact name to a client should also pin the Search
/// companion roots. Subdomain pins (`developer.google.com`) stay surgical.
#[must_use]
pub fn expands_google_search_companions(domain: &str) -> bool {
    domain.eq_ignore_ascii_case("google.com")
}

/// Hosts whose live connections must be closed after a pin apply so they
/// reconnect on the new outbound. Does not log the pin.
#[must_use]
pub fn rebind_hosts_for_pin(pin: &str) -> Vec<String> {
    let pin = pin.trim().trim_end_matches('.').to_ascii_lowercase();
    if pin.is_empty() {
        return Vec::new();
    }
    let mut hosts = vec![pin.clone()];
    if expands_google_search_companions(&pin) {
        for companion in GOOGLE_SEARCH_COMPANION_DOMAINS {
            hosts.push((*companion).into());
        }
    }
    hosts
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_the_google_com_apex_expands() {
        assert!(expands_google_search_companions("google.com"));
        assert!(expands_google_search_companions("GOOGLE.COM"));
        assert!(!expands_google_search_companions("www.google.com"));
        assert!(!expands_google_search_companions("developer.google.com"));
        assert!(!expands_google_search_companions("gstatic.com"));
    }

    #[test]
    fn rebind_for_google_com_includes_search_companions() {
        let hosts = rebind_hosts_for_pin("google.com");
        assert_eq!(hosts[0], "google.com");
        for companion in GOOGLE_SEARCH_COMPANION_DOMAINS {
            assert!(hosts.iter().any(|host| host == companion), "{companion}");
        }
    }

    #[test]
    fn rebind_for_other_pins_is_only_that_host() {
        assert_eq!(
            rebind_hosts_for_pin("developer.google.com"),
            vec!["developer.google.com".to_string()]
        );
    }
}

use crate::{DirectTarget, RuleError};
use psl::{List, Psl};
use std::net::IpAddr;

/// Parses a user pin into an exact (sub)domain or a literal IP.
///
/// The domain is kept exactly as typed (after IDNA normalization): a pin on
/// `google.com` covers every subdomain, while a pin on
/// `developer.google.com` can live in another list and, being more
/// specific, wins over the root pin.
///
/// # Errors
///
/// Returns [`RuleError::InvalidRule`] when the input is not a valid IP,
/// IDNA domain, or sits at/above a public suffix (`co.uk`, `github.io`).
pub fn canonical_target(input: &str) -> Result<DirectTarget, RuleError> {
    let input = input.trim();
    if let Ok(address) = input.parse::<IpAddr>() {
        return Ok(DirectTarget::Ip(address));
    }
    let ascii = crate::normalize_domain(input)?;
    // Still require a registrable root so bare public suffixes cannot be
    // pinned, but keep the exact name the user typed.
    registrable_domain(&ascii)?;
    Ok(DirectTarget::Domain(ascii))
}

/// Labels in a domain; more labels = more specific pin. Used to order
/// generated rules so the longest matching pin wins across lists.
#[must_use]
pub fn domain_specificity(domain: &str) -> usize {
    domain.split('.').count()
}

/// Returns the eTLD+1 for an already-normalized ASCII domain, using the
/// public-suffix list including the private section.
///
/// # Errors
///
/// Returns [`RuleError::InvalidRule`] when the name is only a public suffix
/// (for example `github.io` or `co.uk`) or has no registrable root.
pub fn registrable_domain(ascii: &str) -> Result<String, RuleError> {
    let domain = List.domain(ascii.as_bytes()).ok_or_else(|| {
        RuleError::InvalidRule(
            "domain must have a registrable root; public suffixes cannot be pinned".into(),
        )
    })?;
    let root = std::str::from_utf8(domain.as_bytes())
        .map_err(|_| RuleError::InvalidRule("registrable domain must be valid UTF-8".into()))?;
    Ok(root.to_ascii_lowercase())
}

/// True when `host` is the pin or a subdomain of it.
#[must_use]
pub fn domain_matches_pin(host: &str, pin: &str) -> bool {
    host == pin || host.ends_with(&format!(".{pin}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keeps_subdomains_exact_and_normalizes_case() {
        assert_eq!(
            canonical_target("api.shop.example.com").expect("sub"),
            DirectTarget::Domain("api.shop.example.com".into())
        );
        assert_eq!(
            canonical_target("www.example.com").expect("www"),
            DirectTarget::Domain("www.example.com".into())
        );
        assert_eq!(
            canonical_target("EXAMPLE.COM.").expect("ascii"),
            DirectTarget::Domain("example.com".into())
        );
        assert!(domain_specificity("developer.google.com") > domain_specificity("google.com"));
    }

    #[test]
    fn keeps_multi_part_public_suffixes() {
        assert_eq!(
            canonical_target("api.example.co.uk").expect("co.uk"),
            DirectTarget::Domain("api.example.co.uk".into())
        );
        assert!(canonical_target("co.uk").is_err());
    }

    #[test]
    fn keeps_private_suffix_tenants_separate() {
        assert_eq!(
            canonical_target("user.github.io").expect("pages"),
            DirectTarget::Domain("user.github.io".into())
        );
        assert!(canonical_target("github.io").is_err());
    }

    #[test]
    fn leaves_literal_ips_exact() {
        assert_eq!(
            canonical_target("203.0.113.8").expect("ip"),
            DirectTarget::Ip("203.0.113.8".parse().expect("ip"))
        );
    }

    #[test]
    fn suffix_match_does_not_cross_label_boundaries() {
        assert!(domain_matches_pin("www.example.com", "example.com"));
        assert!(domain_matches_pin("example.com", "example.com"));
        assert!(!domain_matches_pin("notexample.com", "example.com"));
        assert!(!domain_matches_pin("example.com.evil.test", "example.com"));
    }
}

//! Label-to-input matching with tiered priority.
//! Priority: exact > startsWith > contains > fuzzy (Jaro-Winkler ≥ 0.85).
//! Multiple matches at same tier → MultipleMatches error with all candidates.

use super::error::{MatchCandidate, MetaError};
use super::targeting::fuzzy_match_score;

/// Match quality tier — higher is better.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum MatchTier {
    Fuzzy = 1,
    Contains = 2,
    StartsWith = 3,
    Exact = 4,
}

impl MatchTier {
    #[allow(dead_code)]
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Exact => "exact",
            Self::StartsWith => "starts_with",
            Self::Contains => "contains",
            Self::Fuzzy => "fuzzy",
        }
    }

    /// Base confidence for this match tier.
    pub fn confidence(&self) -> f32 {
        match self {
            Self::Exact => 1.0,
            Self::StartsWith => 0.9,
            Self::Contains => 0.75,
            Self::Fuzzy => 0.6,
        }
    }
}

/// Result of a label match operation.
#[derive(Debug, Clone)]
pub struct MatchResult {
    /// Index into the candidate list.
    pub index: usize,
    /// The matched text.
    pub text: String,
    /// Match confidence (0.0-1.0).
    pub confidence: f32,
    /// Which tier matched.
    #[allow(dead_code)]
    pub tier: MatchTier,
    /// Optional element role.
    pub role: Option<String>,
    /// Optional CSS selector.
    pub selector: Option<String>,
}

/// A candidate element for label matching.
#[derive(Debug, Clone)]
pub struct LabelCandidate {
    /// The label/name text of the element.
    pub text: String,
    /// Optional element role.
    pub role: Option<String>,
    /// Optional CSS selector or ref.
    pub selector: Option<String>,
    /// Index for tracking.
    pub index: usize,
}

/// Find the best match for a target label among candidates.
/// Returns Ok(MatchResult) on single best match, Err(MultipleMatches) on ambiguity,
/// Err(ElementNotFound) on no match.
pub fn find_best_match(
    target: &str,
    candidates: &[LabelCandidate],
) -> Result<MatchResult, MetaError> {
    if candidates.is_empty() {
        return Err(MetaError::not_found(target, "form"));
    }

    let target_lower = target.to_lowercase();
    let mut matches: Vec<(MatchTier, MatchResult)> = Vec::new();

    for candidate in candidates {
        let cand_lower = candidate.text.to_lowercase();

        let tier = if cand_lower == target_lower {
            Some(MatchTier::Exact)
        } else if cand_lower.starts_with(&target_lower) || target_lower.starts_with(&cand_lower) {
            Some(MatchTier::StartsWith)
        } else if cand_lower.contains(&target_lower) || target_lower.contains(&cand_lower) {
            Some(MatchTier::Contains)
        } else {
            let score = fuzzy_match_score(target, &candidate.text);
            if score >= 0.85 {
                Some(MatchTier::Fuzzy)
            } else {
                None
            }
        };

        if let Some(tier) = tier {
            matches.push((
                tier,
                MatchResult {
                    index: candidate.index,
                    text: candidate.text.clone(),
                    confidence: tier.confidence(),
                    tier,
                    role: candidate.role.clone(),
                    selector: candidate.selector.clone(),
                },
            ));
        }
    }

    if matches.is_empty() {
        return Err(MetaError::not_found(target, "form"));
    }

    // Find the highest tier
    let best_tier = matches.iter().map(|(t, _)| *t).max().unwrap();

    // Filter to only the highest-tier matches
    let best_matches: Vec<MatchResult> = matches
        .into_iter()
        .filter(|(t, _)| *t == best_tier)
        .map(|(_, m)| m)
        .collect();

    if best_matches.len() == 1 {
        Ok(best_matches.into_iter().next().unwrap())
    } else {
        // Multiple matches at same tier → ambiguity
        let match_candidates: Vec<MatchCandidate> = best_matches
            .iter()
            .map(|m| MatchCandidate {
                text: m.text.clone(),
                role: m.role.clone(),
                selector: m.selector.clone(),
                confidence: m.confidence,
            })
            .collect();
        Err(MetaError::MultipleMatches {
            target: target.to_string(),
            candidates: match_candidates,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_candidates(labels: &[&str]) -> Vec<LabelCandidate> {
        labels
            .iter()
            .enumerate()
            .map(|(i, l)| LabelCandidate {
                text: l.to_string(),
                role: None,
                selector: None,
                index: i,
            })
            .collect()
    }

    #[test]
    fn test_exact_match() {
        let candidates = make_candidates(&["Email", "Password", "Submit"]);
        let result = find_best_match("Email", &candidates).unwrap();
        assert_eq!(result.tier, MatchTier::Exact);
        assert_eq!(result.index, 0);
    }

    #[test]
    fn test_case_insensitive_exact() {
        let candidates = make_candidates(&["Email Address", "Password"]);
        let result = find_best_match("email address", &candidates).unwrap();
        assert_eq!(result.tier, MatchTier::Exact);
    }

    #[test]
    fn test_starts_with() {
        let candidates = make_candidates(&["Email Address", "Password"]);
        let result = find_best_match("Email", &candidates).unwrap();
        assert_eq!(result.tier, MatchTier::StartsWith);
    }

    #[test]
    fn test_contains() {
        let candidates = make_candidates(&["Your Email Address", "Password"]);
        let result = find_best_match("Email", &candidates).unwrap();
        assert_eq!(result.tier, MatchTier::Contains);
    }

    #[test]
    fn test_no_match() {
        let candidates = make_candidates(&["Username", "Password"]);
        let result = find_best_match("Phone Number", &candidates);
        assert!(result.is_err());
    }

    #[test]
    fn test_multiple_matches_same_tier() {
        let candidates = make_candidates(&["Submit Form", "Submit Review", "Cancel"]);
        let result = find_best_match("Submit", &candidates);
        assert!(result.is_err());
        if let Err(MetaError::MultipleMatches { candidates, .. }) = result {
            assert_eq!(candidates.len(), 2);
        } else {
            panic!("Expected MultipleMatches error");
        }
    }

    #[test]
    fn test_empty_candidates() {
        let result = find_best_match("Email", &[]);
        assert!(result.is_err());
    }
}

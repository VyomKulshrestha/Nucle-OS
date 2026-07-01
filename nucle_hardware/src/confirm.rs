//! Confirmation gating: refuses to submit a batch containing any
//! cost-bearing or destructive request unless the caller has explicitly
//! confirmed. This lives in the library, not the CLI, so every consumer of
//! `Provider` gets the same safety check — not just `nucle hardware export`.

use crate::provider::Provider;
use nucle_lang::hardware::HardwareRequest;
use nucle_lang::Effect;

/// True if any request in the batch has a non-`Pure` effect (Synthesis,
/// Sequencing, or Destructive) — i.e. is cost-bearing or destructive.
pub fn is_effectful(batch: &[HardwareRequest]) -> bool {
    batch.iter().any(|r| r.effect != Effect::Pure)
}

/// Number of cost-bearing/destructive requests in the batch.
pub fn count_effectful(batch: &[HardwareRequest]) -> usize {
    batch.iter().filter(|r| r.effect != Effect::Pure).count()
}

/// Submit a batch to `provider`, refusing first — before the provider ever
/// sees it — if the batch contains any cost-bearing/destructive request and
/// `confirmed` is false.
pub fn submit_with_confirmation(
    provider: &dyn Provider,
    batch: &[HardwareRequest],
    confirmed: bool,
) -> Result<String, String> {
    let effectful = count_effectful(batch);
    if effectful > 0 && !confirmed {
        return Err(format!(
            "Refusing to submit {} cost-bearing/destructive request(s) without confirmation.",
            effectful
        ));
    }
    provider.execute_batch(batch)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fixtures::{destructive_request, pure_request, sequencing_request, synthesis_request};
    use crate::mock::MockProvider;

    #[test]
    fn is_effectful_true_for_synthesis() {
        assert!(is_effectful(&[synthesis_request("a.bin")]));
    }

    #[test]
    fn is_effectful_true_for_sequencing() {
        assert!(is_effectful(&[sequencing_request("a.bin")]));
    }

    #[test]
    fn is_effectful_true_for_destructive() {
        assert!(is_effectful(&[destructive_request("a.bin")]));
    }

    #[test]
    fn is_effectful_false_for_all_pure() {
        assert!(!is_effectful(&[pure_request("a.bin"), pure_request("b.bin")]));
    }

    #[test]
    fn is_effectful_false_for_empty_batch() {
        assert!(!is_effectful(&[]));
    }

    #[test]
    fn count_effectful_counts_only_non_pure() {
        let batch = vec![
            pure_request("a.bin"),
            synthesis_request("b.bin"),
            destructive_request("c.bin"),
            pure_request("d.bin"),
        ];
        assert_eq!(count_effectful(&batch), 2);
    }

    #[test]
    fn submit_with_confirmation_rejects_effectful_batch_without_confirm() {
        let provider = MockProvider;
        let batch = [synthesis_request("a.bin")];
        let result = submit_with_confirmation(&provider, &batch, false);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("without confirmation"));
    }

    #[test]
    fn submit_with_confirmation_rejects_destructive_batch_without_confirm() {
        // Destructive operations are the highest-consequence case — verified
        // on its own rather than folded into the generic synthesis test.
        let provider = MockProvider;
        let batch = [destructive_request("archive.bin")];
        let result = submit_with_confirmation(&provider, &batch, false);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains('1'));
    }

    #[test]
    fn submit_with_confirmation_allows_effectful_batch_with_confirm() {
        let provider = MockProvider;
        let batch = [synthesis_request("a.bin"), destructive_request("b.bin")];
        let result = submit_with_confirmation(&provider, &batch, true);
        assert!(result.is_ok());
    }

    #[test]
    fn submit_with_confirmation_allows_pure_batch_without_confirm() {
        let provider = MockProvider;
        let batch = [pure_request("a.bin")];
        let result = submit_with_confirmation(&provider, &batch, false);
        assert!(result.is_ok());
    }

    #[test]
    fn submit_with_confirmation_error_names_the_correct_count() {
        let provider = MockProvider;
        let batch = [synthesis_request("a.bin"), sequencing_request("b.bin"), destructive_request("c.bin")];
        let err = submit_with_confirmation(&provider, &batch, false).unwrap_err();
        assert!(err.contains('3'), "expected error to mention 3 effectful requests, got: {}", err);
    }
}

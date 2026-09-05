// Tests exercise failure paths directly; unwrap/expect, slicing, and
// panicking asserts are the test signal here.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::panic
)]

//! Property-based tests for key validation and store invariants.

use blobkit::memory::MemoryStore;
use blobkit::store::BlobStore;
use blobkit::types::ObjectKey;
use bytes::Bytes;
use proptest::prelude::*;

proptest! {
    #[test]
    fn object_key_rejects_dotdot_component(s in "\\PC*") {
        let has_dotdot_component = s.split('/').any(|c| c == "..");
        if has_dotdot_component {
            prop_assert!(ObjectKey::new(s).is_err());
        }
    }

    #[test]
    fn object_key_max_len_enforced(s in "\\PC{0, 2048}") {
        let len = s.len();
        let res = ObjectKey::new(s);
        if len > 1024 {
            prop_assert!(res.is_err());
        }
    }

    #[test]
    fn object_key_rejects_empty_segments(s in "[a-z]{1,5}(//[a-z]{1,5})+") {
        prop_assert!(ObjectKey::new(s).is_err());
    }

    #[test]
    fn bucket_name_valid_pattern(s in "[a-z0-9]{3,10}(-[a-z0-9]{1,10})*") {
        let res = blobkit::types::BucketName::new(s.clone());
        if s.len() >= 3 && s.len() <= 63
            && s.chars().next().unwrap().is_ascii_alphanumeric()
            && s.chars().last().unwrap().is_ascii_alphanumeric()
            && s.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
        {
            prop_assert!(res.is_ok(), "should accept valid bucket name: {}", s);
        }
    }

    #[test]
    fn memory_put_get_roundtrip_data(data in prop::collection::vec(any::<u8>(), 0..4096)) {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        rt.block_on(async {
            let store = MemoryStore::new();
            let key = ObjectKey::new("proptest/data.bin").unwrap();
            let bytes = Bytes::from(data.clone());
            store.put(key.clone(), bytes.clone()).await.unwrap();
            let got = store.get(&key).await.unwrap();
            prop_assert_eq!(got.as_ref(), data.as_slice());
            Ok::<(), TestCaseError>(())
        }).unwrap();
    }
}

#[test]
fn blob_id_roundtrip() {
    let id = blobkit::types::BlobId::new();
    let s = id.to_string();
    let parsed = s.parse::<blobkit::types::BlobId>().unwrap();
    assert_eq!(id, parsed);
}

// Copyright 2024-2026 WritersLogic Contributors
// SPDX-License-Identifier: Apache-2.0

use super::*;

impl WarmCore {
    /// Query the audit log for entries since `timestamp_ms`.
    /// Returns an empty vec if audit logging is disabled.
    pub fn audit_since(&self, timestamp_ms: u64) -> Result<Vec<super::super::audit::AuditEntry>> {
        let Some(audit) = &self.audit else {
            return Ok(Vec::new());
        };
        let mut entries = audit.entries_since(timestamp_ms)?;
        if let Some(stream) = &self.audit_effects {
            entries.extend(stream.entries_since(timestamp_ms)?);
        }
        Ok(entries)
    }

    #[cfg(feature = "provenance")]
    pub(super) fn provenance_manager(
        &self,
    ) -> Result<&super::super::provenance::ProvenanceManager> {
        self.provenance
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("provenance not enabled"))
    }

    #[cfg(feature = "provenance")]
    pub fn provenance_enabled(&self) -> bool {
        self.provenance.is_some()
    }

    #[cfg(feature = "provenance")]
    pub fn create_fact_provenance(
        &self,
        fact_id: &str,
        content: &[u8],
        source_uri: Option<&str>,
    ) -> Result<super::super::provenance::types::ProvenanceRecord> {
        let mgr = self.provenance_manager()?;
        mgr.create_fact_provenance(fact_id, content, source_uri)
    }

    #[cfg(feature = "provenance")]
    pub fn create_triple_provenance(
        &self,
        params: &super::super::provenance::TripleProvenanceParams<'_>,
    ) -> Result<super::super::provenance::types::ProvenanceRecord> {
        let mgr = self.provenance_manager()?;
        mgr.create_triple_provenance(params)
    }

    #[cfg(feature = "provenance")]
    pub fn create_store_manifest(
        &self,
        store_id: &str,
        store_data: &[u8],
        fact_count: usize,
        dimensions: u32,
        title: Option<&str>,
    ) -> Result<super::super::provenance::types::StoreManifest> {
        let mgr = self.provenance_manager()?;
        mgr.create_store_manifest(store_id, store_data, fact_count, dimensions, title)
    }

    #[cfg(feature = "provenance")]
    pub fn verify_fact_provenance(
        &self,
        record: &super::super::provenance::types::ProvenanceRecord,
        trust: &super::super::provenance::trust::TrustStore,
    ) -> Result<super::super::provenance::types::VerificationResult> {
        let mgr = self.provenance_manager()?;
        mgr.verify_fact_provenance(record, trust)
    }

    /// A trust store containing this instance's own provenance key, for
    /// authenticating records it signed. `None` if provenance is disabled.
    #[cfg(feature = "provenance")]
    pub fn provenance_self_trust(&self) -> Option<super::super::provenance::trust::TrustStore> {
        self.provenance.as_ref().map(|mgr| mgr.self_trust())
    }

    #[cfg(feature = "provenance")]
    pub fn verify_store_manifest(
        &self,
        manifest: &super::super::provenance::types::StoreManifest,
        store_data: Option<&[u8]>,
        trust: &super::super::provenance::trust::TrustStore,
    ) -> Result<super::super::provenance::types::VerificationResult> {
        let mgr = self.provenance_manager()?;
        mgr.verify_store_manifest(manifest, store_data, trust)
    }

    #[cfg(feature = "provenance")]
    pub fn issuer_did(&self) -> Option<&str> {
        self.provenance.as_ref().map(|mgr| mgr.issuer_did())
    }

    #[cfg(feature = "provenance")]
    pub fn get_provenance(
        &self,
        fact_id: &str,
    ) -> Option<super::super::provenance::types::ProvenanceRecord> {
        self.provenance.as_ref()?.get_record(fact_id)
    }

    #[cfg(feature = "provenance")]
    pub fn provenance_count(&self) -> usize {
        self.provenance.as_ref().map_or(0, |mgr| mgr.record_count())
    }

    #[cfg(feature = "provenance")]
    pub fn create_self_manifest(
        &self,
        title: Option<&str>,
    ) -> Result<super::super::provenance::types::StoreManifest> {
        let mgr = self.provenance_manager()?;
        let store_data =
            std::fs::read(self.storage_path.join("vectors_data.bin")).unwrap_or_default();
        let store_id = self
            .storage_path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("default");
        mgr.create_store_manifest(
            store_id,
            &store_data,
            self.vector_count() as usize,
            self.dimensions as u32,
            title,
        )
    }

    #[cfg(feature = "provenance")]
    pub fn revoke_credential(&self, status_index: u64) -> Result<()> {
        let mgr = self.provenance_manager()?;
        mgr.revoke_credential(status_index)
    }

    #[cfg(feature = "provenance")]
    pub fn status_list_credential(
        &self,
    ) -> Result<super::super::provenance::status_list::StatusListCredential> {
        self.provenance_manager()?.status_list_credential()
    }

    #[cfg(feature = "provenance")]
    pub fn is_credential_revoked(&self, status_index: u64) -> bool {
        self.provenance
            .as_ref()
            .is_some_and(|mgr| mgr.is_revoked(status_index))
    }

    #[cfg(feature = "provenance")]
    pub fn verify_provenance_log(&self) -> Result<bool> {
        let mgr = self.provenance_manager()?;
        mgr.verify_log_integrity()
    }

    #[cfg(feature = "provenance")]
    pub fn provenance_log_integrity(&self) -> Result<super::super::provenance::LogIntegrity> {
        self.provenance_manager()?.log_integrity()
    }

    #[cfg(feature = "provenance")]
    pub fn create_batch_provenance(
        &self,
        items: &[(&str, &[u8], Option<&str>)],
    ) -> Result<Vec<super::super::provenance::types::ProvenanceRecord>> {
        let mgr = self.provenance_manager()?;
        mgr.create_batch_provenance(items)
    }

    #[cfg(feature = "provenance")]
    pub fn create_sigstore_bundle(
        &self,
        content: &[u8],
        identity: Option<&str>,
    ) -> Result<super::super::provenance::sigstore::SigstoreBundle> {
        let mgr = self.provenance_manager()?;
        mgr.create_sigstore_bundle(content, identity)
    }

    #[cfg(feature = "provenance")]
    pub fn verify_sigstore_bundle(
        &self,
        bundle: &super::super::provenance::sigstore::SigstoreBundle,
        content: &[u8],
        trusted_key: &ed25519_dalek::VerifyingKey,
    ) -> Result<()> {
        let mgr = self.provenance_manager()?;
        mgr.verify_sigstore_bundle(bundle, content, trusted_key)
    }

    #[cfg(feature = "provenance")]
    pub fn create_cawg_assertion(
        &self,
        referenced: &[(String, String, Vec<u8>)],
        display_name: &str,
    ) -> Result<serde_json::Value> {
        let mgr = self.provenance_manager()?;
        mgr.create_cawg_assertion(referenced, display_name)
    }

    #[cfg(feature = "provenance")]
    pub fn verify_cawg_assertion(
        &self,
        assertion: &serde_json::Value,
        trust: &super::super::provenance::trust::TrustStore,
    ) -> Result<serde_json::Value> {
        let mgr = self.provenance_manager()?;
        mgr.verify_cawg_assertion(assertion, trust)
    }
}

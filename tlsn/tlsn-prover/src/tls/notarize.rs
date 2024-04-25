//! This module implements the attestation protocol for the prover.

use futures::{FutureExt, SinkExt, StreamExt};
use rand::{thread_rng, Rng};
use tlsn_common::{attestation::AttestationRequest, msg::TlsnMessage};
use tlsn_core::{
    attestation::{AttestationBodyBuilder, AttestationFull, FieldData, Secret},
    conn::{CertificateSecrets, ConnectionInfo, ServerIdentity, TlsVersion},
    encoding::{EncodingCommitment, EncodingTree},
    substring::SubstringCommitConfigBuilder,
    transcript::Transcript,
};
#[cfg(feature = "tracing")]
use tracing::instrument;
use utils_aio::{expect_msg_or_err, mux::MuxChannel};

use crate::tls::{
    convert_mpc_tls_data, error::OTShutdownError, ff::ShareConversionReveal, state::Notarize,
    Prover, ProverError,
};

impl Prover<Notarize> {
    /// Returns a reference to the transcript.
    pub fn transcript(&self) -> &Transcript {
        &self.state.transcript
    }

    /// Returns a mutable reference to the substring commitment builder.
    pub fn substring_commitment_builder(&mut self) -> &mut SubstringCommitConfigBuilder {
        &mut self.state.substring_commitment_builder
    }

    /// Finalize the notarization returning an [`AttestationFull`].
    #[cfg_attr(feature = "tracing", instrument(level = "info", skip(self), err))]
    pub async fn finalize(self) -> Result<AttestationFull, ProverError> {
        let Notarize {
            mut mux_ctrl,
            mut mux_fut,
            mut vm,
            mut ot_fut,
            mut gf2,
            start_time,
            mpc_tls_data,
            transcript,
            encoding_provider,
            substring_commitment_builder,
        } = self.state;

        let (hs_data, cert_data) = convert_mpc_tls_data(mpc_tls_data);

        let conn_info = ConnectionInfo {
            time: start_time,
            version: TlsVersion::V1_2,
            transcript_length: transcript.length(),
        };

        let cert_data = CertificateSecrets {
            data: cert_data,
            cert_nonce: thread_rng().gen(),
            chain_nonce: thread_rng().gen(),
        };

        let cert_commitment = cert_data
            .cert_commitment(self.config.field_commitment_alg())
            .expect("certificate chain is present");
        let cert_chain_commitment = cert_data
            .cert_chain_commitment(self.config.field_commitment_alg())
            .expect("certificate chain is present");

        let substring_commitment_config = substring_commitment_builder.build().unwrap();

        let encoding_tree = if substring_commitment_config.has_encoding() {
            Some(
                EncodingTree::new(
                    *substring_commitment_config.encoding_hash_alg(),
                    substring_commitment_config.iter_encoding(),
                    &encoding_provider,
                    &transcript.length(),
                )
                .unwrap(),
            )
        } else {
            None
        };

        let request = AttestationRequest {
            hash_alg: self.config.attestation_hash_alg(),
            time: start_time,
            cert_commitment: cert_commitment.clone(),
            cert_chain_commitment: cert_chain_commitment.clone(),
            encoding_commitment_root: encoding_tree.as_ref().map(|tree| tree.root()),
            extra_data: vec![],
        };

        let mut notarize_fut = Box::pin(async move {
            let mut channel = mux_ctrl.get_channel("notarize").await?;

            channel
                .send(TlsnMessage::AttestationRequest(request))
                .await?;

            let notary_encoder_seed = vm
                .finalize()
                .await
                .map_err(|e| ProverError::MpcError(Box::new(e)))?
                .expect("encoder seed returned");

            // This is a temporary approach until a maliciously secure share conversion protocol is implemented.
            // The prover is essentially revealing the TLS MAC key. In some exotic scenarios this allows a malicious
            // TLS verifier to modify the prover's request.
            gf2.reveal()
                .await
                .map_err(|e| ProverError::MpcError(Box::new(e)))?;

            let signed_attestation = expect_msg_or_err!(channel, TlsnMessage::SignedAttestation)?;

            Ok::<_, ProverError>((notary_encoder_seed, signed_attestation))
        })
        .fuse();

        let (notary_encoder_seed, signed_attestation) = futures::select_biased! {
            res = notarize_fut => res?,
            _ = ot_fut => return Err(OTShutdownError)?,
            _ = &mut mux_fut => return Err(std::io::Error::from(std::io::ErrorKind::UnexpectedEof))?,
        };
        // Wait for the notary to correctly close the connection
        mux_fut.await?;

        let mut attestation_body_builder = AttestationBodyBuilder::default();
        attestation_body_builder
            .field(FieldData::ConnectionInfo(conn_info))
            .field(FieldData::HandshakeData(hs_data))
            .field(FieldData::CertificateCommitment(cert_commitment))
            .field(FieldData::CertificateChainCommitment(cert_chain_commitment));

        if let Some(encoding_tree) = &encoding_tree {
            attestation_body_builder.field(FieldData::EncodingCommitment(EncodingCommitment {
                root: encoding_tree.root(),
                seed: notary_encoder_seed.to_vec(),
            }));
        }

        let attestation_body = attestation_body_builder.build().unwrap();

        // Make sure the Notary signed the correct root hash.
        if &attestation_body.root(self.config.attestation_hash_alg())
            != &signed_attestation.header.root
        {
            todo!()
        }

        let mut secrets = vec![
            Secret::Certificate(cert_data),
            Secret::ServerIdentity(ServerIdentity::new(self.config.server_dns().to_string())),
        ];

        if let Some(encoding_tree) = encoding_tree {
            secrets.push(Secret::EncodingTree(encoding_tree));
        }

        let attestation_full = AttestationFull {
            sig: signed_attestation.sig,
            header: signed_attestation.header,
            body: attestation_body,
            transcript,
            secrets,
        };

        Ok(attestation_full)
    }
}

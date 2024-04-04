//! Traits for the prover backend and the verifier backend.

use crate::{
    prover::error::ProverError,
    verifier::{error::VerifierError, verifier::VerificationInputs},
    Proof, ProofInput,
};
use num::{BigInt, BigUint};
use serde::{Deserialize, Serialize};
use std::ops::{Add, Sub};

/// A trait for zk proof generation backend.
pub trait ProverBackend<F>
where
    F: Field,
{
    /// Creates a commitment to the plaintext, padding the plaintext if necessary.
    ///
    /// Returns the commitment and the salt used to create the commitment.
    fn commit_plaintext(&self, plaintext: Vec<bool>) -> Result<(F, F), ProverError>;

    /// Creates a commitment to the encoding sum.
    ///
    /// Returns the commitment and the salt used to create the commitment.
    fn commit_encoding_sum(&self, encoding_sum: F) -> Result<(F, F), ProverError>;

    /// Given the `input` to the AuthDecode zk circuit, generates and returns `Proof`(s)
    fn prove(&self, input: Vec<ProofInput<F>>) -> Result<Vec<Proof>, ProverError>;

    /// How many bits of [Plaintext] can fit into one [Chunk]. This does not
    /// include the [Salt] of the hash - which takes up the remaining least bits
    /// of the last field element of each chunk.
    fn chunk_size(&self) -> usize;
}

/// A trait for zk proof verification backend.
pub trait VerifierBackend<F>
where
    F: Field,
{
    /// Verifies multiple inputs against multiple proofs.
    /// Which inputs correspond to which proof is determined internally by the backend.
    fn verify(
        &self,
        inputs: Vec<VerificationInputs<F>>,
        proofs: Vec<Proof>,
    ) -> Result<(), VerifierError>;

    /// How many bits of [Plaintext] can fit into one [Chunk]. This does not
    /// include the [Salt] of the hash - which takes up the remaining least bits
    /// of the last field element of each chunk.
    fn chunk_size(&self) -> usize;
}

/// Methods to work with a field element.
pub trait Field {
    /// Creates a new field element from bytes in big-endian byte order.
    fn from_bytes_be(bytes: Vec<u8>) -> Self;

    /// Returns zero, the additive identity.
    fn zero() -> Self;

    /// Return inner bytes
    fn inner(&self) -> Vec<u8>;
}

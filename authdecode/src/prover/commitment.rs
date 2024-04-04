use crate::{
    backend::traits::{Field, ProverBackend as Backend},
    bitid::IdSet,
    encodings::{
        active::ActiveEncodingsChunks,
        state::{Converted, Original},
        ActiveEncodings, Encoding,
    },
};
use num::{BigInt, BigUint, FromPrimitive};

use super::error::ProverError;

/// The plaintext and the encodings which the prover commits to.
pub struct CommitmentData<T>
where
    T: IdSet,
{
    pub encodings: ActiveEncodings<T, Original>,
}

impl<T> CommitmentData<T>
where
    T: IdSet,
{
    /// Creates a commitment to this data.
    pub fn commit<F>(
        &self,
        backend: &Box<dyn Backend<F>>,
    ) -> Result<CommitmentDetails<T, F>, ProverError>
    where
        F: Field + Clone + std::ops::Add<Output = F>,
    {
        // Chunk up the data and commit to each chunk individually.
        let chunk_commitments = self
            .into_chunks(backend.chunk_size())
            .map(|data_chunk| data_chunk.commit(backend))
            .collect::<Result<Vec<ChunkCommitmentDetails<T, F>>, ProverError>>()?;

        Ok(CommitmentDetails { chunk_commitments })
    }

    /// Creates a new `CommitmentData` type for `plaintext` with the given bit ids. Bits encode to
    /// `encodings`.
    ///
    /// # Panics
    ///
    /// Panics if data, encodings and ids are not all of the same length.
    pub fn new(plaintext: Vec<bool>, encodings: Vec<Vec<u8>>, bit_ids: T) -> CommitmentData<T> {
        assert!(plaintext.len() == encodings.len());
        assert!(plaintext.len() == bit_ids.len());

        let encodings = plaintext
            .iter()
            .zip(encodings)
            .map(|(bit, enc)| Encoding::new(enc, *bit))
            .collect::<Vec<_>>();

        CommitmentData {
            encodings: ActiveEncodings::new(encodings, bit_ids),
        }
    }

    pub fn into_chunks(&self, chunk_size: usize) -> CommitmentDataChunks<T> {
        CommitmentDataChunks {
            encodings: self.encodings.clone().into_chunks(chunk_size),
        }
    }
}

pub struct CommitmentDataChunks<T> {
    encodings: ActiveEncodingsChunks<T, Original>,
}

impl<T> Iterator for CommitmentDataChunks<T>
where
    T: IdSet,
{
    type Item = CommitmentDataChunk<T>;

    fn next(&mut self) -> Option<Self::Item> {
        self.encodings
            .next()
            .map(|encodings| Some(CommitmentDataChunk { encodings }))?
    }
}

// A chunk of data that needs to be committed to.
pub struct CommitmentDataChunk<T>
where
    T: IdSet,
{
    pub encodings: ActiveEncodings<T, Original>,
}

impl<T> CommitmentDataChunk<T>
where
    T: IdSet,
{
    /// Creates a commitment to this chunk.
    fn commit<F>(
        &self,
        backend: &Box<dyn Backend<F>>,
    ) -> Result<ChunkCommitmentDetails<T, F>, ProverError>
    where
        F: Field + Clone + std::ops::Add<Output = F>,
    {
        // Convert the encodings and compute their sum.
        let encodings = self.encodings.convert();
        let sum = encodings.compute_sum::<F>();
        println!("Encoding sum clear: {:x?}", sum.inner());

        let (plaintext_hash, plaintext_salt) = backend.commit_plaintext(encodings.plaintext())?;

        let (encoding_sum_hash, encoding_sum_salt) = backend.commit_encoding_sum(sum.clone())?;

        Ok(ChunkCommitmentDetails {
            plaintext_hash,
            plaintext_salt,
            original_encodings: self.encodings.clone(),
            encodings,
            encoding_sum: sum,
            encoding_sum_hash,
            encoding_sum_salt,
        })
    }
}

/// An AuthDecode commitment to a single chunk of the plaintext with the associated details.
#[derive(Clone)]
pub struct ChunkCommitmentDetails<T, F>
where
    T: IdSet,
    F: Field,
{
    pub plaintext_hash: F,
    pub plaintext_salt: F,

    /// The original (i.e. before conversion) encodings of the plaintext bits.
    pub original_encodings: ActiveEncodings<T, Original>,
    // The converted (i.e. uncorrelated and truncated) encodings to commit to.
    pub encodings: ActiveEncodings<T, Converted>,

    pub encoding_sum: F,
    pub encoding_sum_hash: F,
    pub encoding_sum_salt: F,
}

impl<T, F> ChunkCommitmentDetails<T, F>
where
    T: IdSet,
    F: Field,
{
    /// Returns the id of each bit of the plaintext.
    pub fn ids(&self) -> &T {
        &self.original_encodings.ids()
    }
}

/// An AuthDecode commitment to plaintext of arbitrary length with the associated details.
#[derive(Clone, Default)]
pub struct CommitmentDetails<T, F>
where
    T: IdSet,
    F: Field + Clone,
{
    /// Commitments to each chunk of the plaintext with the associated details.
    ///
    /// Internally, for performance reasons, the data to be committed to is split up into chunks
    /// and each chunk is committed to separately. The collection of chunk commitments constitutes
    /// the commitment.
    pub chunk_commitments: Vec<ChunkCommitmentDetails<T, F>>,
}

impl<T, F> CommitmentDetails<T, F>
where
    T: IdSet + Clone,
    F: Field + Clone,
{
    /// Returns the original encodings of the plaintext of this commitment.
    pub fn original_encodings(&self) -> ActiveEncodings<T, Original> {
        let iter = self
            .chunk_commitments
            .iter()
            .map(|enc| enc.original_encodings.clone());
        ActiveEncodings::new_from_iter(iter)
    }
}

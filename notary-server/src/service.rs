pub mod axum_websocket;
pub mod tcp;
pub mod websocket;

use async_trait::async_trait;
use async_tungstenite::tungstenite::handshake::server;
use axum::{
    extract::{rejection::JsonRejection, FromRequestParts, Query, State},
    http::{header, request::Parts, StatusCode},
    response::{IntoResponse, Json, Response},
    Error,
};
use axum_macros::debug_handler;
use chrono::Utc;

use p256::ecdsa::{Signature, SigningKey};
use tlsn_verifier::tls::{Verifier, VerifierConfig};
use tokio::io::{AsyncRead, AsyncWrite};
use tokio_util::compat::TokioAsyncReadCompatExt;
use tracing::{debug, error, info, trace};
use uuid::Uuid;

use crate::{
    domain::notary::{
        NotarizationRequestQuery, NotarizationSessionRequest, NotarizationSessionResponse,
        NotaryGlobals, SessionData, TLSProof, VerificationRequest,
    },
    error::NotaryServerError,
    service::{
        axum_websocket::{header_eq, WebSocketUpgrade},
        tcp::{tcp_notarize, TcpUpgrade},
        websocket::websocket_notarize,
    },
};

/// A wrapper enum to facilitate extracting TCP connection for either WebSocket or TCP clients,
/// so that we can use a single endpoint and handler for notarization for both types of clients
pub enum ProtocolUpgrade {
    Tcp(TcpUpgrade),
    Ws(WebSocketUpgrade),
}

#[async_trait]
impl<S> FromRequestParts<S> for ProtocolUpgrade
where
    S: Send + Sync,
{
    type Rejection = NotaryServerError;

    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, Self::Rejection> {
        // Extract tcp connection for websocket client
        if header_eq(&parts.headers, header::UPGRADE, "websocket") {
            let extractor = WebSocketUpgrade::from_request_parts(parts, state)
                .await
                .map_err(|err| NotaryServerError::BadProverRequest(err.to_string()))?;
            return Ok(Self::Ws(extractor));
        // Extract tcp connection for tcp client
        } else if header_eq(&parts.headers, header::UPGRADE, "tcp") {
            let extractor = TcpUpgrade::from_request_parts(parts, state)
                .await
                .map_err(|err| NotaryServerError::BadProverRequest(err.to_string()))?;
            return Ok(Self::Tcp(extractor));
        } else {
            return Err(NotaryServerError::BadProverRequest(
                "Upgrade header is not set for client".to_string(),
            ));
        }
    }
}

/// Handler to upgrade protocol from http to either websocket or underlying tcp depending on the type of client
/// the session_id parameter is also extracted here to fetch the configuration parameters
/// that have been submitted in the previous request to /session made by the same client
pub async fn upgrade_protocol(
    protocol_upgrade: ProtocolUpgrade,
    State(notary_globals): State<NotaryGlobals>,
    Query(params): Query<NotarizationRequestQuery>,
) -> Response {
    info!("Received upgrade protocol request");
    let session_id = params.session_id;
    // Fetch the configuration data from the store using the session_id
    // This also removes the configuration data from the store as each session_id can only be used once
    let (max_sent_data, max_recv_data) = match notary_globals.store.lock().await.remove(&session_id)
    {
        Some(data) => (data.max_sent_data, data.max_recv_data),
        None => {
            let err_msg = format!("Session id {} does not exist", session_id);
            error!(err_msg);
            return NotaryServerError::BadProverRequest(err_msg).into_response();
        }
    };
    // This completes the HTTP Upgrade request and returns a successful response to the client, meanwhile initiating the websocket or tcp connection
    match protocol_upgrade {
        ProtocolUpgrade::Ws(ws) => ws.on_upgrade(move |socket| {
            websocket_notarize(
                socket,
                notary_globals,
                session_id,
                max_sent_data,
                max_recv_data,
            )
        }),
        ProtocolUpgrade::Tcp(tcp) => tcp.on_upgrade(move |stream| {
            tcp_notarize(
                stream,
                notary_globals,
                session_id,
                max_sent_data,
                max_recv_data,
            )
        }),
    }
}

pub async fn initialize(
    State(notary_globals): State<NotaryGlobals>,
    payload: Result<Json<NotarizationSessionRequest>, JsonRejection>,
) -> impl IntoResponse {
    info!(
        ?payload,
        "Received request for initializing a notarization session"
    );

    // Parse the body payload
    let payload = match payload {
        Ok(payload) => payload,
        Err(err) => {
            error!("Malformed payload submitted for initializing notarization: {err}");
            return NotaryServerError::BadProverRequest(err.to_string()).into_response();
        }
    };

    // Ensure that the max_transcript_size submitted is not larger than the global max limit configured in notary server
    if payload.max_sent_data.is_some() || payload.max_recv_data.is_some() {
        let requested_transcript_size =
            payload.max_sent_data.unwrap_or_default() + payload.max_recv_data.unwrap_or_default();
        if requested_transcript_size > notary_globals.notarization_config.max_transcript_size {
            error!(
                "Max transcript size requested {:?} exceeds the maximum threshold {:?}",
                requested_transcript_size, notary_globals.notarization_config.max_transcript_size
            );
            return NotaryServerError::BadProverRequest(
                "Max transcript size requested exceeds the maximum threshold".to_string(),
            )
            .into_response();
        }
    }

    let prover_session_id = Uuid::new_v4().to_string();

    // Store the configuration data in a temporary store
    notary_globals.store.lock().await.insert(
        prover_session_id.clone(),
        SessionData {
            max_sent_data: payload.max_sent_data,
            max_recv_data: payload.max_recv_data,
            created_at: Utc::now(),
        },
    );

    trace!("Latest store state: {:?}", notary_globals.store);

    // Return the session id in the response to the client
    (
        StatusCode::OK,
        Json(NotarizationSessionResponse {
            session_id: prover_session_id,
        }),
    )
        .into_response()
}

use tlsn_core::{
    proof::{SessionProof, SubstringsProof, TlsProof},
    session::SessionHeader,
    transcript,
};

/// Proof that a transcript of communications took place between a Prover and Server.
#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub struct VerifyProofRequest {
    /// Proof of the TLS handshake, server identity, and commitments to the transcript.
    pub auth_proof: TLSProof,
    /// Proof of the user attributes
    pub attribute_proof: TLSProof,
}

/// Handler to verify the TLS proof and sign it with EDDSA
#[debug_handler(state = NotaryGlobals)]
pub async fn verify_proof(
    State(notary_globals): State<NotaryGlobals>,
    payload: Result<Json<VerifyProofRequest>, JsonRejection>,
) -> impl IntoResponse {
    info!("📨 Received request to verify TLS proof");

    // Parse the body payload and extract TLSProof
    let payload: VerifyProofRequest = match payload {
        Ok(Json(payload)) => payload,
        Err(err) => {
            error!("Malformed payload submitted for initializing notarization: {err}");
            return NotaryServerError::BadProverRequest(err.to_string()).into_response();
        }
    };

    //info!("payload: {:#?}", payload);
    let (signature, nullifier, claim_key) = verify(payload).await.unwrap();

    // Return a JSON with field success = "OK" in the response to the client
    (
        StatusCode::OK,
        Json(serde_json::json!({
            "success": "OK",
            "signature": signature.to_string(),
            "nullifier": nullifier,
            "claim_key": claim_key,
        })),
    )
        .into_response()
}

use super::airdrop;
use std::time::Duration;
#[derive(serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VerifyResult {
    pub server_name: String,
    pub time: u64,
    pub sent: String,
    pub recv: String,
}

/// Parses authentication and attribute proofs from TLSProof structures
///
/// # Arguments
/// * `auth_proof` - TLSProof for authentication
/// * `attribute_proof` - TLSProof for attributes
///
/// # Returns
/// A tuple containing parsed data for both proofs:
/// ((auth_header, auth_server_name, auth_substrings), (attr_header, attr_server_name, attr_substrings))
pub fn parse_proofs(
    auth_proof: TLSProof,
    attribute_proof: TLSProof,
) -> (
    (SessionHeader, String, SubstringsProof),
    (SessionHeader, String, SubstringsProof),
) {
    // Extract authentication proof components
    let TLSProof {
        session: auth_session,
        substrings: auth_substrings,
    } = auth_proof;
    let SessionProof {
        header: auth_header,
        session_info: auth_session_info,
        ..
    } = auth_session;
    let auth_server_name = String::from(auth_session_info.server_name.as_str());

    // Extract attribute proof components
    let TLSProof {
        session: attr_session,
        substrings: attr_substrings,
    } = attribute_proof;
    let SessionProof {
        header: attr_header,
        session_info: attr_session_info,
        ..
    } = attr_session;
    let attr_server_name = String::from(attr_session_info.server_name.as_str());

    // Return parsed components
    (
        (auth_header, auth_server_name, auth_substrings),
        (attr_header, attr_server_name, attr_substrings),
    )
}

/// Verifies the provided TLS proof using the notary's public key.
///
/// # Arguments
///
/// * `proof` - The TLS proof to be verified.
/// * `notary_pubkey_str` - The notary's public key as a string.
///
/// # Returns
///
/// A result containing a tuple with the Ed25519 signature and a message that can be empty
/// or an error if the verification fails.
///

pub async fn verify(request: VerifyProofRequest) -> Result<(String, Vec<u8>, String), NotaryServerError> {
    let (
        (auth_header, auth_server_name, auth_substrings),
        (attr_header, attr_server_name, attr_substrings),
    ) = parse_proofs(request.auth_proof, request.attribute_proof);

    // verify that proofs are from same server
    if attr_server_name != auth_server_name {
        return Err(NotaryServerError::BadProverRequest(
            "Server names do not match".to_string(),
        ));
    }

    // @TODO verify tls certificates
    // session
    //     .verify_with_default_cert_verifier(get_notary_pubkey(notary_pubkey_str)?)
    //     .map_err(|e| &format!("Session verification failed: {:?}", e))?;

    //@TEST to uncomment in production
    //let time = chrono::DateTime::UNIX_EPOCH + Duration::from_secs(auth_header.time());
    // Verify that the session is not older than 24 hours
    // let current_time = chrono::Utc::now().timestamp() as u64;
    // let session_time = header.time();
    // let time_difference = current_time.saturating_sub(session_time);
    // const TWENTY_FOUR_HOURS_IN_SECONDS: u64 = 24 * 60 * 60;
    // if time_difference > TWENTY_FOUR_HOURS_IN_SECONDS {
    //     return Err(NotaryServerError::BadProverRequest(
    //         "Session is older than 24 hours".to_string(),
    //     ));
    // }

    let (mut auth_sent, mut auth_recv) = auth_substrings.verify(&auth_header).unwrap();
    let (mut attr_sent, mut attr_recv) = attr_substrings.verify(&attr_header).unwrap();

    // Replace the bytes which the Prover chose not to disclose with 'X'
    attr_recv.set_redacted(b'X');
    auth_recv.set_redacted(b'X');

    //@Note uncomment to verify that fields have been hidden
    // info!("-------------------------------------------------------------------");
    // info!(
    //     "Successfully verified that the bytes below came from a session with {:?} at {}.",
    //     session_info.server_name, time
    // );
    // info!("Bytes sent:");
    // info!(
    //     "{}",
    //     String::from_utf8(sent.data().to_vec())
    //         .unwrap_or("Could not convert sent data to string".to_string())
    // );
    // info!("Bytes received:");
    // info!(
    //     "{}",
    //     String::from_utf8(attr_recv.data().to_vec())
    //         .unwrap_or("Could not convert recv data to string".to_string())
    // );
    // info!(
    //     "{}",
    //     String::from_utf8(auth_recv.data().to_vec())
    //         .unwrap_or("Could not convert recv data to string".to_string())
    // );
    // info!("-------------------------------------------------------------------");

    // @DEBUG : remove dummyjson
    // if it's kaggle, we will parse user_id from transcript, check dedup then return an auth_signature
    if auth_server_name == "www.kaggle.com" || auth_server_name == "dummyjson.com" {
        let res = airdrop::generate_signature_userid(
            auth_recv,
            attr_recv,
            auth_server_name,
            &attr_header.merkle_root(),
        )
        .await;
        return match res {
            Ok((signature, nullifier, claim_key)) => Ok((signature, nullifier, claim_key)),
            Err(e) => Err(NotaryServerError::BadProverRequest(e.to_string())),
        };
    } else {
        return Err(NotaryServerError::BadProverRequest(format!(
            "Server '{}' is not in the list of supported servers",
            auth_server_name
        )));
    }
}

/// Run the notarization
pub async fn notary_service<T: AsyncWrite + AsyncRead + Send + Unpin + 'static>(
    socket: T,
    signing_key: &SigningKey,
    session_id: &str,
    max_sent_data: Option<usize>,
    max_recv_data: Option<usize>,
) -> Result<(), NotaryServerError> {
    debug!(?session_id, "Starting notarization...");

    let mut config_builder = VerifierConfig::builder();

    config_builder = config_builder.id(session_id);

    debug!("config_builder.max_sent_data {:?}", max_sent_data);

    if let Some(max_sent_data) = max_sent_data {
        config_builder = config_builder.max_sent_data(max_sent_data);
    }

    if let Some(max_recv_data) = max_recv_data {
        config_builder = config_builder.max_recv_data(max_recv_data);
    }

    debug!("config_builder.build");

    let config = config_builder.build()?;

    debug!("Verifier::new");

    let verifier = Verifier::new(config);

    verifier
        .notarize::<_, Signature>(socket.compat(), signing_key)
        .await?;

    Ok(())
}

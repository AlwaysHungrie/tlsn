//! TLS Airdrop
//!
use p256::pkcs8::der::asn1::Int;
use reqwest::Response;
use serde_json::Number;
use std::collections::HashMap;
use std::time::{Duration, Instant};

use std::env;
use tlsn_core::{
    msg::{SessionTranscripts, SignedSessionHeader, TlsnMessage},
    HandshakeSummary, SessionHeader, Signature, Transcript,
};
use tracing::info;
use uuid::Uuid;

const MIN_FOLLOWERS: u64 = 0;

const AIRDROP_SERVER: &str = "https://airdrop-server.fly.dev";

#[allow(non_snake_case)]
#[derive(serde::Deserialize, Debug)]
struct RespFollowers {
    userId: u64,
}
#[allow(non_snake_case)]
#[derive(serde::Deserialize, Debug)]
struct RespProfile {
    displayName: String,
    userId: u64,
    usersFollowingMe: Vec<RespFollowers>,
}

#[allow(non_snake_case)]
#[derive(serde::Deserialize, Debug)]
struct RespClaimInsert {
    success: bool,
}

#[derive(serde::Deserialize, Debug)]
#[allow(non_snake_case)]
struct Claim {
    id: u64,
    user_id: String,
    website: String,
    claim_key: String,
    claimed: bool,
}

#[derive(serde::Deserialize, Debug)]
struct RespClaimView {
    claims: Vec<Claim>,
}
#[allow(non_snake_case)]
#[derive(serde::Deserialize, Debug)]
struct RespKaggle {
    userProfile: RespProfile,
}

impl RespKaggle {
    fn new() -> RespKaggle {
        RespKaggle {
            userProfile: RespProfile {
                displayName: String::new(),
                userId: 0,
                usersFollowingMe: Vec::new(),
            },
        }
    }
}

/// Parses the session transcripts to extract the host and user ID.
///
/// # Arguments
///
/// * `session_transcripts` - The session transcripts containing the transmitted and received data.
///
/// # Returns
///
/// A tuple containing the host and user ID as strings.
pub fn parse_transcripts(sent: String, rcv: String) -> (String, String) {
    // Convert the transmitted and received transcripts to strings

    // Define the keys to search for in the received transcript to extract the user ID
    let start_key = String::from("userName\":\"");
    let end_key = String::from("\"");
    let user_id: String = parse_value(rcv, start_key, end_key);

    // Define the keys to search for in the transmitted transcript to extract the host
    let start_key = String::from("host: ");
    let end_key = String::from("\r\n");
    let host: String = parse_value(sent, start_key, end_key);

    // Return the extracted host and user ID as a tuple
    return (host, user_id);
}

/// Parses a value from a string based on start and end keys.
///
/// # Arguments
///
/// * `str` - The string to parse the value from.
/// * `start_key` - The key indicating the start of the value.
/// * `end_key` - The key indicating the end of the value.
///
/// # Returns
///
/// The parsed value as a string. If the value cannot be found, an empty string is returned.
pub fn parse_value(str: String, start_key: String, end_key: String) -> String {
    let key = String::from(start_key);

    let parsed_value: String = match str.find(&key) {
        Some(start_pos) => {
            let start = start_pos + key.len();
            let end_pos = str[start..].find(&end_key).unwrap();
            str[start..start + end_pos].to_string()
        }
        err => {
            println!("error parsing value from transcript");
            println!("{:?}", err);
            "".to_string()
            //panic()! uncomment in production
        }
    };
    parsed_value
}

/// Inserts a claim key for a user on a specific host.
///
/// # Arguments
///
/// * `user_id` - The ID of the user.
/// * `host` - The host website.
/// * `uuid` - The claim key to be inserted.
///
/// # Returns
///
/// A boolean indicating whether the claim key was successfully inserted.
pub async fn insert_claim_key(user_id: String, host: String, uuid: String) -> bool {
    info!("host {:?} user_id: {:?} uuid {:?}", host, user_id, uuid);

    if host != "www.kaggle.com" {
        return false;
    }

    let client = reqwest::Client::new();

    let mut map = HashMap::new();
    map.insert("claim_key", uuid);
    map.insert("user_id", user_id);
    //map.insert("user_id", "test".to_string());
    map.insert("website", host);

    let url = format!("{:}/insert-claim-key", AIRDROP_SERVER);
    let airdrop_server_auth = std::env::var("AIRDROP_SERVER_AUTH").unwrap();
    let res = client
        .post(url)
        .header("Authorization", airdrop_server_auth)
        .json(&map)
        .send()
        .await
        .unwrap();

    println!("status = {:?}", res.status());

    let resp_claim_insert: RespClaimInsert = res.json().await.unwrap();
    println!("res = {:#?}", resp_claim_insert);

    return resp_claim_insert.success;
}

/// Views the claim key for a user.
///
/// # Arguments
///
/// * `user_id` - The ID of the user.
///
/// # Returns
///
/// A tuple containing a boolean indicating whether a claim key exists and the claim key as a string.
pub async fn view_claim_key(user_id: String) -> (bool, String) {
    let client = reqwest::Client::new();

    let mut map = HashMap::new();
    map.insert("user_id", user_id);

    let url = format!("{:}/view-user-claims", AIRDROP_SERVER);
    let airdrop_server_auth = std::env::var("AIRDROP_SERVER_AUTH").unwrap();
    let res = client
        .post(url)
        .header("Authorization", airdrop_server_auth)
        .json(&map)
        .send()
        .await
        .unwrap();

    println!("status = {:?}", res.status());

    let resp_claim_insert: RespClaimView = res.json().await.unwrap();
    println!("res = {:#?}", resp_claim_insert);

    if resp_claim_insert.claims.len() > 0 {
        return (true, resp_claim_insert.claims[0].claim_key.clone());
    } else {
        return (false, "".to_string());
    }
}
/// Checks the number of followers for a given user.
///
/// # Arguments
///
/// * `user_id` - The ID of the user.
///
/// # Returns
///
/// A boolean indicating whether the user has the minimum required followers.
pub async fn check_followers(user_id: String) -> bool {
    let client = reqwest::Client::new();

    let mut map = HashMap::new();
    map.insert("relativeUrl", user_id.clone());

    let res = client
            .post("https://www.kaggle.com/api/i/routing.RoutingService/GetPageDataByUrl")
            .header("cookie", "ka_sessionid=6cff08a3142d89f9fe8e8232d101f5ec; CSRF-TOKEN=CfDJ8CHCUm6ypKVLpjizcZHPE706CGhBGw-qXt3fYKSnshHAHCz7JZRraz7CY0pF39jTcccPTjfh7sKqyoPMZ8DtjiKzjpJzophmKaNKY_cv2A; GCLB=CJD19dbEidGQ0wEQAw; build-hash=25329b9ee1e8ff6e9268ed171e37e91972f190cf; recaptcha-ca-t=AaGzOmdJKOWu-htf89JEBvCCVQMG1SteZS4dMNVE4o06Djc4hrVQSWeV1ygz4ZzvkaWwqviyUdt40OzDxW4K0-twsw_6UvvBtInLAWKsWhSNHMmVE7E3ddo0YPNkdvaLsaNkIMPDtZ8csqHM6g:U=e480c09ba0000000; XSRF-TOKEN=CfDJ8CHCUm6ypKVLpjizcZHPE70HA0syy35mtn6KbUjCbOddkpiyjjo1c-dvBq0e71nnCYWEOLl6qRVufWFyh5GeEdnzdiM-ZcrEz4EboI5lussb4w; CLIENT-TOKEN=eyJhbGciOiJub25lIiwidHlwIjoiSldUIn0.eyJpc3MiOiJrYWdnbGUiLCJhdWQiOiJjbGllbnQiLCJzdWIiOiIiLCJuYnQiOiIyMDI0LTA2LTE3VDE4OjA5OjI2LjkxMjczNzVaIiwiaWF0IjoiMjAyNC0wNi0xN1QxODowOToyNi45MTI3Mzc1WiIsImp0aSI6ImEwMWZjNWNkLTA0YjctNDFjMS05NjNmLTJiNDE2YWIxZjIwNSIsImV4cCI6IjIwMjQtMDctMTdUMTg6MDk6MjYuOTEyNzM3NVoiLCJhbm9uIjp0cnVlLCJmZiI6WyJLZXJuZWxzRmlyZWJhc2VMb25nUG9sbGluZyIsIkFsbG93Rm9ydW1BdHRhY2htZW50cyIsIkZyb250ZW5kRXJyb3JSZXBvcnRpbmciLCJSZWdpc3RyYXRpb25OZXdzRW1haWxTaWdudXBJc09wdE91dCIsIkRpc2N1c3Npb25zUmVhY3Rpb25zIiwiRGF0YXNldFVwbG9hZGVyRHVwbGljYXRlRGV0ZWN0aW9uIiwiRGF0YXNldHNMbG1GZWVkYmFja0NoaXAiLCJNZXRhc3RvcmVDaGVja0FnZ3JlZ2F0ZUZpbGVIYXNoZXMiLCJLTU1hdGVyaWFsVUlEaWFsb2ciLCJBbGxSb3V0ZXNUb1JlYWN0Um91dGVyIl0sImZmZCI6eyJLZXJuZWxFZGl0b3JBdXRvc2F2ZVRocm90dGxlTXMiOiIzMDAwMCIsIkVtZXJnZW5jeUFsZXJ0QmFubmVyIjoie30iLCJDbGllbnRScGNSYXRlTGltaXRRcHMiOiI0MCIsIkNsaWVudFJwY1JhdGVMaW1pdFFwbSI6IjUwMCIsIkZlYXR1cmVkQ29tbXVuaXR5Q29tcGV0aXRpb25zIjoiNjAwOTUsNTQwMDAsNTcxNjMsODA4NzQiLCJBZGRGZWF0dXJlRmxhZ3NUb1BhZ2VMb2FkVGFnIjoiZGlzYWJsZWQiLCJNb2RlbElkc0FsbG93SW5mZXJlbmNlIjoiMzMwMSwzNTMzIiwiTW9kZWxJbmZlcmVuY2VQYXJhbWV0ZXJzIjoieyBcIm1heF90b2tlbnNcIjogMTI4LCBcInRlbXBlcmF0dXJlXCI6IDAuNCwgXCJ0b3Bfa1wiOiA1IH0iLCJDb21wZXRpdGlvbk1ldHJpY1RpbWVvdXRNaW51dGVzIjoiMzAifSwicGlkIjoia2FnZ2xlLTE2MTYwNyIsInN2YyI6IndlYi1mZSIsInNkYWsiOiJBSXphU3lBNGVOcVVkUlJza0pzQ1pXVnotcUw2NTVYYTVKRU1yZUUiLCJibGQiOiIyNTMyOWI5ZWUxZThmZjZlOTI2OGVkMTcxZTM3ZTkxOTcyZjE5MGNmIn0.")
            .header("x-xsrf-token", "CfDJ8CHCUm6ypKVLpjizcZHPE70HA0syy35mtn6KbUjCbOddkpiyjjo1c-dvBq0e71nnCYWEOLl6qRVufWFyh5GeEdnzdiM-ZcrEz4EboI5lussb4w")
            .json(&map)
            .send()
            .await;

    let followers = match res {
        Ok(res) => {
            println!("status = {:?}", res.status());
            //assert!(res.status() == 200, "failed to retrieve user attributes");

            let resp_kaggle = RespKaggle::new();
            let val: RespKaggle = res.json().await.unwrap_or(resp_kaggle);

            let followers: u64 = val
                .userProfile
                .usersFollowingMe
                .len()
                .try_into()
                .unwrap_or(0);
            followers
        }
        Err(err) => {
            //info!("error when querying kaggle attributes {:}", err);
            0
            //panic!("request to kaggle failed");
        }
    };

    println!(" {:?} followers > {:?}", followers, MIN_FOLLOWERS);

    return followers >= MIN_FOLLOWERS;

    //info!("result = {:?}", result);
}
#[cfg(feature = "tracing")]
mod test {
    use super::*;
    // use serde::Serialize;
    // use serde_json::json;

    #[test]
    #[cfg(feature = "tracing")]
    fn test_parsing() {
        let json_str = String::from(
            r#"
    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nDate: Fri, 14 Jun 2024 02:51:49 GMT\r\nTransfer-Encoding: chunked\r\nX-Frame-Options: SAMEORIGIN\r\nStrict-Transport-Security: max-age=63072000; includeSubDomains; preload\r\nContent-Security-Policy: object-src 'none'; script-src 'nonce-ZUToT69xQ40F4JPtCyvLZw==' 'report-sample' 'unsafe-inline' 'unsafe-eval' 'strict-dynamic' https: http:; base-uri 'none'; report-uri https://csp.withgoogle.com/csp/kaggle/20201130; frame-src 'self' https://www.kaggleusercontent.com https://www.youtube.com/embed/ https://polygraph-cool.github.io https://www.google.com/recaptcha/ https://www.docdroid.com https://www.docdroid.net https://kaggle-static.storage.googleapis.com https://kkb-production.jupyter-proxy.kaggle.net https://kkb-production.firebaseapp.com https://kaggle-metastore.firebaseapp.com https://apis.google.com https://content-sheets.googleapis.com/ https://accounts.google.com/ https://storage.googleapis.com https://docs.google.com https://drive.google.com https://calendar.google.com/;\r\nX-Content-Type-Options: nosniff\r\nReferrer-Policy: strict-origin-when-cross-origin\r\nVia: 1.1 google\r\nAlt-Svc: h3=\":443\"; ma=2592000,h3-29=\":443\"; ma=2592000\r\nConnection: close\r\n\r\n192\r\n{\"id\":21142885,\"displayName\":\"Zlim93200\",\"email\":\"batchtrain@gmail.com\",\"userName\":\"zlim93200\",\"thumbnailUrl\":\"https://storage.googleapis.com/kaggle-avatars/thumbnails/default-thumb.png\",\"profileUrl\":\"/zlim93200\",\"registerDate\":\"2024-06-04T16:22:44.700Z\",\"lastVisitDate\":\"2024-06-14T02:36:09.207Z\",\"statusId\":2,\"canAct\":true,\"canBeSeen\":true,\"thumbnailName\":\"default-thumb.png\",\"httpAcceptLanguage\":\"\"}\r\n0\r\n\r\n"
    "#,
        );

        // \"userName\":\"zlim93200\"
        let start_key = String::from("userName\\\":\\\"");
        let end_key = String::from("\\\",");

        let parsed_value: String = parse_value(json_str, start_key, end_key);

        println!("parsed_value: {}", parsed_value);
        assert!(parsed_value == "zlim93200")
    }

    #[tokio::test]
    #[cfg(feature = "tracing")]
    async fn test_insert_claim_key() {
        let user_id = "Zlim93200".to_string().to_lowercase();
        let host = "www.kaggle.com".to_string();
        let claim_token = "token123".to_string();

        let resp = insert_claim_key(user_id, host, claim_token).await;
        println!("{resp:#?}");
    }

    #[tokio::test]
    #[cfg(feature = "tracing")]
    async fn test_view_claim_key() {
        let user_id = "Zlim93200".to_string().to_lowercase();

        let resp = view_claim_key(user_id).await;
        println!("{resp:#?}");
    }

    #[tokio::test]
    #[cfg(feature = "tracing")]
    async fn test_check_followers() {
        let user_id = "Zlim93200".to_string();
        let result = check_followers(user_id).await;
        println!("result = {:?}", result);
        //assert!(result == 42, "Failed to grant claim token");
    }

    #[tokio::test]
    #[cfg(feature = "tracing")]
    async fn test_flow() {
        let user_id = "Zlim93200".to_string();
        let host = "www.kaggle.com".to_string();
        //let claim_token = "token123".to_string();
        let uuid = Uuid::new_v4().to_string();

        let is_valid = check_followers(user_id.clone()).await;
        println!("is_valid = {:?}", is_valid);

        if is_valid {
            let inserted = insert_claim_key(user_id, host, uuid).await;
            println!("inserted = {:?}", inserted);
        }
        //assert!(result == 42, "Failed to grant claim token");
    }
}
use super::HelperServiceError;
use tokio::net::UnixStream;
use tracing::warn;

pub(crate) fn authenticated_uid(
    stream: &UnixStream,
    authorized_uid: u32,
) -> Result<Option<u32>, HelperServiceError> {
    let peer_uid = stream.peer_cred()?.uid();
    if uid_is_authorized(peer_uid, authorized_uid) {
        return Ok(Some(peer_uid));
    }
    warn!(
        event = "helper.peer_rejected",
        section = "helper_security",
        initiator = "ipc_peer",
        cause = "unauthorized_uid",
        trace_route = "ipc_peer->credential_check->reject",
        peer_uid,
        "rejected unauthorized helper peer"
    );
    Ok(None)
}

fn uid_is_authorized(peer_uid: u32, authorized_uid: u32) -> bool {
    peer_uid == authorized_uid || peer_uid == 0
}

#[cfg(test)]
mod tests {
    use super::{authenticated_uid, uid_is_authorized};
    use tokio::net::UnixStream;

    #[test]
    fn sharing_a_group_does_not_authorize_another_uid() {
        assert!(uid_is_authorized(501, 501));
        assert!(uid_is_authorized(0, 501));
        assert!(!uid_is_authorized(502, 501));
    }

    #[tokio::test]
    async fn socket_identity_comes_from_the_kernel() {
        let (stream, _peer) = UnixStream::pair().expect("socket pair");
        let uid = nix::unistd::geteuid().as_raw();
        assert_eq!(
            authenticated_uid(&stream, uid).expect("credentials"),
            Some(uid)
        );
        if uid != 0 {
            assert_eq!(
                authenticated_uid(&stream, uid + 1).expect("credentials"),
                None
            );
        }
    }
}

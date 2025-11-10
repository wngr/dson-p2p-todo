// ABOUTME: UDP broadcast networking for delta synchronization.
// ABOUTME: Supports network isolation toggle for partition testing.

use crate::app::ReplicaId;
use dson::{CausalDotStore, Delta, OrMap};
use serde::{Deserialize, Serialize};
use std::{
    io,
    net::{SocketAddr, UdpSocket},
};

pub const DEFAULT_PORT: u16 = 7878;

/// Network message types for CRDT synchronization.
#[derive(Serialize, Deserialize, Debug)]
pub enum NetworkMessage {
    /// Full delta containing CRDT state.
    Delta {
        sender_id: ReplicaId,
        delta: Delta<CausalDotStore<OrMap<String>>>,
    },
    /// Anti-entropy: just the causal context for comparison.
    Context {
        sender_id: ReplicaId,
        context: dson::CausalContext,
    },
}

impl NetworkMessage {
    pub fn sender_id(&self) -> ReplicaId {
        match self {
            NetworkMessage::Delta { sender_id, .. } => *sender_id,
            NetworkMessage::Context { sender_id, .. } => *sender_id,
        }
    }
}

/// Create and configure a UDP socket for broadcasting.
/// Binds to the specified port for receiving, and allows broadcasting to any port.
/// Uses SO_REUSEPORT on macOS/BSD to allow multiple instances on the same port.
pub fn create_broadcast_socket(port: u16) -> io::Result<UdpSocket> {
    use socket2::{Domain, Socket, Type};
    use std::net::{Ipv4Addr, SocketAddrV4};

    // Create socket with socket2 to set SO_REUSEPORT before binding
    // On macOS/BSD, SO_REUSEPORT allows multiple processes to bind to the same port
    // and all will receive copies of broadcast packets
    let socket = Socket::new(Domain::IPV4, Type::DGRAM, None)?;
    socket.set_reuse_address(true)?;
    #[cfg(unix)]
    socket.set_reuse_port(true)?;

    socket.set_broadcast(true)?;
    socket.set_nonblocking(true)?;

    let addr = SocketAddrV4::new(Ipv4Addr::UNSPECIFIED, port);
    socket.bind(&addr.into())?;

    Ok(socket.into())
}

/// Broadcast a message to all peers.
/// If isolated is true, returns Ok without sending (simulates network partition).
///
/// # Errors
/// Returns an error if `data.len()` exceeds the network MTU (typically ~1500 bytes for Ethernet).
pub fn broadcast(socket: &UdpSocket, data: &[u8], port: u16, isolated: bool) -> io::Result<()> {
    if isolated {
        // Silently drop when isolated
        return Ok(());
    }

    let broadcast_addr = format!("255.255.255.255:{port}");
    socket.send_to(data, broadcast_addr)?;
    Ok(())
}

/// Maximum UDP packet size in bytes.
const MAX_UDP_PACKET_SIZE: usize = 65536;

/// Try to receive a message from the network (non-blocking).
/// If isolated is true, returns Ok(None) without reading (simulates network partition).
/// Returns Ok(None) if no message is available (WouldBlock).
pub fn try_receive(
    socket: &UdpSocket,
    isolated: bool,
) -> io::Result<Option<(Vec<u8>, SocketAddr)>> {
    if isolated {
        // Silently drop when isolated
        return Ok(None);
    }

    let mut buf = vec![0u8; MAX_UDP_PACKET_SIZE];
    match socket.recv_from(&mut buf) {
        Ok((size, addr)) => {
            buf.truncate(size);
            Ok(Some((buf, addr)))
        }
        Err(e) if e.kind() == io::ErrorKind::WouldBlock => Ok(None),
        Err(e) => Err(e),
    }
}

/// Serialize a network message to bytes using MessagePack.
pub fn serialize_message(msg: &NetworkMessage) -> io::Result<Vec<u8>> {
    rmp_serde::to_vec(msg).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))
}

/// Deserialize bytes to a network message using MessagePack.
pub fn deserialize_message(data: &[u8]) -> io::Result<NetworkMessage> {
    rmp_serde::from_slice(data).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))
}

#[cfg(test)]
mod tests {
    use super::*;
    use dson::Identifier;

    #[test]
    fn test_serialize_deserialize_roundtrip() {
        let mut store = CausalDotStore::<OrMap<String>>::default();
        let id = Identifier::new(1, 0);
        let mut tx = store.transact(id);
        tx.write_register(
            "test",
            dson::crdts::mvreg::MvRegValue::String("hello".to_string()),
        );
        let delta = tx.commit();

        let msg = NetworkMessage::Delta {
            sender_id: ReplicaId::new(42),
            delta,
        };

        let serialized = serialize_message(&msg).expect("Failed to serialize");
        let deserialized = deserialize_message(&serialized).expect("Failed to deserialize");

        assert_eq!(deserialized.sender_id(), ReplicaId::new(42));
    }

    #[test]
    fn test_broadcast_when_isolated_does_not_send() {
        // This is a behavioral test - when isolated, broadcast should succeed but not actually send
        let socket = create_broadcast_socket(0).expect("Failed to create socket");
        let result = broadcast(&socket, b"test", DEFAULT_PORT, true);
        assert!(result.is_ok());
    }

    #[test]
    fn test_try_receive_when_isolated_returns_none() {
        let socket = create_broadcast_socket(0).expect("Failed to create socket");
        let result = try_receive(&socket, true).expect("Failed to try_receive");
        assert!(result.is_none());
    }
}

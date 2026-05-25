use std::net::SocketAddr;
use std::time::{SystemTime, UNIX_EPOCH};

pub struct Sender {
    socket: tokio::net::UdpSocket,
    target: SocketAddr,
}

pub struct Receiver {
    socket: tokio::net::UdpSocket,
}

impl Sender {
    pub async fn bind(local: &str, target: &str) -> anyhow::Result<Self> {
        let socket = tokio::net::UdpSocket::bind(local).await?;
        let target: SocketAddr = target.parse()?;
        Ok(Self { socket, target })
    }

    /// Send periodic synthetic "frames" as simple UDP packets with a header
    pub async fn run(mut self) -> anyhow::Result<()> {
        let mut seq: u64 = 0;
        loop {
            seq += 1;
            let ts = SystemTime::now().duration_since(UNIX_EPOCH)?.as_millis() as u64;
            let mut buf = Vec::with_capacity(16 + 4);
            buf.extend_from_slice(&seq.to_be_bytes());
            buf.extend_from_slice(&ts.to_be_bytes());
            // payload: small synthetic frame id
            buf.extend_from_slice(&[0u8, 1, 2, 3]);
            let _ = self.socket.send_to(&buf, &self.target).await?;
            tokio::time::sleep(std::time::Duration::from_millis(33)).await; // ~30fps
        }
    }
}

impl Receiver {
    pub async fn bind(local: &str) -> anyhow::Result<Self> {
        let socket = tokio::net::UdpSocket::bind(local).await?;
        Ok(Self { socket })
    }

    /// Run receiver loop printing basic stats.
    pub async fn run(self) -> anyhow::Result<()> {
        let mut buf = vec![0u8; 1500];
        let mut last_seq: Option<u64> = None;
        loop {
            let (n, addr) = self.socket.recv_from(&mut buf).await?;
            if n < 12 {
                eprintln!("received too-small packet from {}", addr);
                continue;
            }
            let seq = u64::from_be_bytes(buf[0..8].try_into().unwrap());
            let ts = u64::from_be_bytes(buf[8..16].try_into().unwrap());
            let payload = &buf[16..n];
            let now = SystemTime::now().duration_since(UNIX_EPOCH)?.as_millis() as u64;
            let rtt = now.saturating_sub(ts);
            let lost = match last_seq {
                Some(prev) if seq > prev + 1 => seq - prev - 1,
                _ => 0,
            };
            last_seq = Some(seq);
            println!("pkt {} from {} payload={} rtt={}ms lost_since_last={}", seq, addr, payload.len(), rtt, lost);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn sender_receiver_smoke() -> anyhow::Result<()> {
        let recv = Receiver::bind("127.0.0.1:50010").await?;
        let sender = Sender::bind("127.0.0.1:0", "127.0.0.1:50010").await?;
        // Run receiver in background
        tokio::spawn(async move { let _ = recv.run().await; });
        // send a few packets then return
        tokio::spawn(async move { let _ = tokio::time::timeout(std::time::Duration::from_millis(200), sender.run()).await; });
        Ok(())
    }
}

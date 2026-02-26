use std::sync::Arc;
use tokio::sync::broadcast;

use super::types::ContentLakeEvent;

/// In-process event bus backed by `tokio::broadcast`.
/// Single-node; will be extended to PG LISTEN/NOTIFY for multi-node.
#[derive(Debug, Clone)]
pub struct EventBus {
    sender: Arc<broadcast::Sender<ContentLakeEvent>>,
}

impl EventBus {
    /// Create a new event bus with the given channel capacity.
    pub fn new(capacity: usize) -> Self {
        let (sender, _) = broadcast::channel(capacity);
        Self {
            sender: Arc::new(sender),
        }
    }

    /// Publish an event to all current subscribers.
    pub fn publish(&self, event: ContentLakeEvent) -> Result<usize, broadcast::error::SendError<ContentLakeEvent>> {
        self.sender.send(event)
    }

    /// Subscribe to the event stream.
    pub fn subscribe(&self) -> broadcast::Receiver<ContentLakeEvent> {
        self.sender.subscribe()
    }

    /// Number of active subscribers.
    pub fn subscriber_count(&self) -> usize {
        self.sender.receiver_count()
    }
}

impl Default for EventBus {
    fn default() -> Self {
        Self::new(1024)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn publish_and_receive() {
        let bus = EventBus::new(16);
        let mut rx = bus.subscribe();

        bus.publish(ContentLakeEvent::Welcome).unwrap();

        let event = rx.recv().await.unwrap();
        assert!(matches!(event, ContentLakeEvent::Welcome));
    }

    #[tokio::test]
    async fn multiple_subscribers() {
        let bus = EventBus::new(16);
        let mut rx1 = bus.subscribe();
        let mut rx2 = bus.subscribe();

        assert_eq!(bus.subscriber_count(), 2);

        bus.publish(ContentLakeEvent::Reconnect).unwrap();

        assert!(matches!(rx1.recv().await.unwrap(), ContentLakeEvent::Reconnect));
        assert!(matches!(rx2.recv().await.unwrap(), ContentLakeEvent::Reconnect));
    }
}

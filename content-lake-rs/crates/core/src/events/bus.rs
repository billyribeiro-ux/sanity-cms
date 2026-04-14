//! Event + presence buses.
//!
//! Each bus is an in-process `tokio::broadcast::Sender` paired with an
//! optional Postgres `LISTEN/NOTIFY` bridge. When wired with a `PgPool`, the
//! bus spawns a background task that issues `LISTEN` on the relevant channel
//! and forwards every incoming payload into the in-process broadcaster.
//! Publishers use [`EventBus::publish_durable`] / [`PresenceBus::publish_durable`]
//! which BOTH broadcast locally (zero-latency fast path) and fire a
//! `pg_notify(channel, payload)` so other cluster nodes see the event.
//!
//! # Dedupe
//!
//! Every event carries a `node_id` stamped by the publishing node. The
//! LISTEN side skips messages whose `node_id` equals our own — we already
//! delivered them locally, so forwarding them a second time would produce
//! duplicate events.
//!
//! # Payload-size fallback
//!
//! Postgres `NOTIFY` payloads are hard-capped at 8000 bytes. Most mutation
//! events fit, but asset-upload events carry the wire document and can
//! easily exceed the limit. When the JSON payload is oversized, we publish
//! a slimmed-down variant that omits the `result` body; SSE listeners on
//! remote nodes will then re-fetch the document by id if they need it.

use std::sync::Arc;
use std::time::Duration;

use sqlx::postgres::PgListener;
use sqlx::PgPool;
use tokio::sync::broadcast;
use uuid::Uuid;

use super::types::{ContentLakeEvent, DocumentMutationEvent, PresenceEvent};

/// Channel name used for `LISTEN/NOTIFY` of document mutation events.
pub const EVENT_CHANNEL: &str = "content_lake_events";

/// Channel name used for `LISTEN/NOTIFY` of presence events.
pub const PRESENCE_CHANNEL: &str = "content_lake_presence";

/// Postgres hard limit on the size of a `NOTIFY` payload, minus a small
/// safety margin for the protocol overhead. Payloads above this are published
/// in a slim form that omits large bodies.
const MAX_NOTIFY_BYTES: usize = 7500;

/// In-process event bus backed by `tokio::broadcast`, optionally bridged to
/// Postgres `LISTEN/NOTIFY` for cross-node delivery.
#[derive(Debug, Clone)]
pub struct EventBus {
    sender: Arc<broadcast::Sender<ContentLakeEvent>>,
    node_id: Uuid,
}

impl EventBus {
    /// Create a new in-process-only event bus with the given channel capacity.
    ///
    /// Events published via [`EventBus::publish`] reach only subscribers in
    /// this process. Suitable for tests or single-node deployments; for
    /// multi-node, use [`EventBus::with_postgres`].
    pub fn new(capacity: usize) -> Self {
        let (sender, _) = broadcast::channel(capacity);
        Self {
            sender: Arc::new(sender),
            node_id: Uuid::nil(),
        }
    }

    /// Create an event bus wired to Postgres `LISTEN/NOTIFY`.
    ///
    /// Spawns a tokio task that listens on [`EVENT_CHANNEL`] and forwards
    /// every incoming event into the in-process broadcaster. Events whose
    /// `node_id` equals `node_id` are skipped — those are our own echoes,
    /// already delivered synchronously by [`EventBus::publish_durable`].
    pub async fn with_postgres(
        capacity: usize,
        pool: &PgPool,
        node_id: Uuid,
    ) -> Result<Self, sqlx::Error> {
        let (sender, _) = broadcast::channel(capacity);
        let sender = Arc::new(sender);
        let bus = Self {
            sender: sender.clone(),
            node_id,
        };

        spawn_listen_bridge::<ContentLakeEvent>(
            pool.clone(),
            EVENT_CHANNEL,
            node_id,
            sender,
            |ev| ev.origin_node_id(),
        );

        Ok(bus)
    }

    /// Broadcast an event to all in-process subscribers. No cross-node
    /// delivery.
    pub fn publish(
        &self,
        event: ContentLakeEvent,
    ) -> Result<usize, broadcast::error::SendError<ContentLakeEvent>> {
        self.sender.send(event)
    }

    /// Broadcast locally AND fan out to other cluster nodes via
    /// `pg_notify`. The local broadcast is synchronous (no DB round-trip);
    /// the NOTIFY is best-effort. NOTIFY errors are logged at WARN and
    /// swallowed so a committed mutation is never failed by a downstream
    /// event-bus hiccup.
    pub async fn publish_durable(
        &self,
        pool: &PgPool,
        mut event: ContentLakeEvent,
    ) -> Result<(), sqlx::Error> {
        // Stamp our node_id so receiving nodes (including ourselves) can
        // dedupe.
        stamp_event_node_id(&mut event, self.node_id);

        // Local broadcast first — this is the latency-sensitive path and
        // must not be gated on the NOTIFY succeeding.
        let _ = self.sender.send(event.clone());

        // Fire NOTIFY. Use the slim variant if the JSON is over the Postgres
        // payload cap.
        let payload = match serde_json::to_string(&event) {
            Ok(s) if s.len() <= MAX_NOTIFY_BYTES => s,
            Ok(oversized) => {
                tracing::debug!(
                    bytes = oversized.len(),
                    "event payload exceeds NOTIFY limit; publishing slim variant"
                );
                match serde_json::to_string(&slim_event(&event)) {
                    Ok(s) => s,
                    Err(e) => {
                        tracing::warn!(error = %e, "failed to serialize slim event; skipping NOTIFY");
                        return Ok(());
                    }
                }
            }
            Err(e) => {
                tracing::warn!(error = %e, "failed to serialize event; skipping NOTIFY");
                return Ok(());
            }
        };

        if let Err(e) = sqlx::query("SELECT pg_notify($1, $2)")
            .bind(EVENT_CHANNEL)
            .bind(&payload)
            .execute(pool)
            .await
        {
            tracing::warn!(error = %e, "pg_notify event publish failed");
        }

        Ok(())
    }

    /// Subscribe to the event stream.
    pub fn subscribe(&self) -> broadcast::Receiver<ContentLakeEvent> {
        self.sender.subscribe()
    }

    /// Number of active subscribers.
    pub fn subscriber_count(&self) -> usize {
        self.sender.receiver_count()
    }

    /// Cluster-local node id stamped into outgoing events.
    pub fn node_id(&self) -> Uuid {
        self.node_id
    }
}

impl Default for EventBus {
    fn default() -> Self {
        Self::new(1024)
    }
}

/// Dedicated broadcast channel for presence (cursor) events. Kept separate
/// from the document mutation bus so a chatty presence stream never backs up
/// the data-layer listeners.
#[derive(Debug, Clone)]
pub struct PresenceBus {
    sender: Arc<broadcast::Sender<PresenceEvent>>,
    node_id: Uuid,
}

impl PresenceBus {
    pub fn new(capacity: usize) -> Self {
        let (sender, _) = broadcast::channel(capacity);
        Self {
            sender: Arc::new(sender),
            node_id: Uuid::nil(),
        }
    }

    /// Presence bus wired to Postgres `LISTEN/NOTIFY` on
    /// [`PRESENCE_CHANNEL`].
    pub async fn with_postgres(
        capacity: usize,
        pool: &PgPool,
        node_id: Uuid,
    ) -> Result<Self, sqlx::Error> {
        let (sender, _) = broadcast::channel(capacity);
        let sender = Arc::new(sender);
        let bus = Self {
            sender: sender.clone(),
            node_id,
        };

        spawn_listen_bridge::<PresenceEvent>(
            pool.clone(),
            PRESENCE_CHANNEL,
            node_id,
            sender,
            |ev| ev.origin_node_id(),
        );

        Ok(bus)
    }

    pub fn publish(
        &self,
        event: PresenceEvent,
    ) -> Result<usize, broadcast::error::SendError<PresenceEvent>> {
        self.sender.send(event)
    }

    /// Broadcast locally AND fan out via `pg_notify`. See
    /// [`EventBus::publish_durable`].
    pub async fn publish_durable(
        &self,
        pool: &PgPool,
        mut event: PresenceEvent,
    ) -> Result<(), sqlx::Error> {
        if let PresenceEvent::State(s) = &mut event {
            s.node_id = self.node_id;
        }

        let _ = self.sender.send(event.clone());

        let payload = match serde_json::to_string(&event) {
            Ok(s) if s.len() <= MAX_NOTIFY_BYTES => s,
            Ok(oversized) => {
                // Presence payloads almost never exceed 8KB — but if a
                // client sends a massive selection blob, drop the selection
                // rather than fail NOTIFY.
                tracing::debug!(
                    bytes = oversized.len(),
                    "presence payload exceeds NOTIFY limit; publishing without selection"
                );
                match serde_json::to_string(&slim_presence(&event)) {
                    Ok(s) => s,
                    Err(e) => {
                        tracing::warn!(error = %e, "failed to serialize slim presence; skipping NOTIFY");
                        return Ok(());
                    }
                }
            }
            Err(e) => {
                tracing::warn!(error = %e, "failed to serialize presence event; skipping NOTIFY");
                return Ok(());
            }
        };

        if let Err(e) = sqlx::query("SELECT pg_notify($1, $2)")
            .bind(PRESENCE_CHANNEL)
            .bind(&payload)
            .execute(pool)
            .await
        {
            tracing::warn!(error = %e, "pg_notify presence publish failed");
        }

        Ok(())
    }

    pub fn subscribe(&self) -> broadcast::Receiver<PresenceEvent> {
        self.sender.subscribe()
    }

    pub fn subscriber_count(&self) -> usize {
        self.sender.receiver_count()
    }

    pub fn node_id(&self) -> Uuid {
        self.node_id
    }
}

impl Default for PresenceBus {
    fn default() -> Self {
        Self::new(1024)
    }
}

/// Spawn a background task that LISTENs on `channel`, deserializes each
/// payload as `T`, and forwards the result into `sender`. Messages whose
/// origin node_id matches `self_node_id` are dropped (they are our own
/// echoes).
///
/// On disconnect the task reconnects with exponential backoff, capped at 30s.
fn spawn_listen_bridge<T>(
    pool: PgPool,
    channel: &'static str,
    self_node_id: Uuid,
    sender: Arc<broadcast::Sender<T>>,
    origin: fn(&T) -> Option<Uuid>,
) where
    T: for<'de> serde::Deserialize<'de> + Clone + Send + Sync + 'static,
{
    tokio::spawn(async move {
        let mut backoff = Duration::from_millis(250);
        let max_backoff = Duration::from_secs(30);

        loop {
            match run_listen_loop::<T>(&pool, channel, self_node_id, &sender, origin).await {
                Ok(()) => {
                    tracing::warn!(channel, "LISTEN loop exited cleanly; reconnecting");
                }
                Err(e) => {
                    tracing::warn!(
                        channel,
                        error = %e,
                        backoff_ms = backoff.as_millis() as u64,
                        "LISTEN loop failed; retrying"
                    );
                }
            }

            tokio::time::sleep(backoff).await;
            backoff = (backoff * 2).min(max_backoff);
        }
    });
}

async fn run_listen_loop<T>(
    pool: &PgPool,
    channel: &str,
    self_node_id: Uuid,
    sender: &broadcast::Sender<T>,
    origin: fn(&T) -> Option<Uuid>,
) -> Result<(), sqlx::Error>
where
    T: for<'de> serde::Deserialize<'de> + Clone,
{
    let mut listener = PgListener::connect_with(pool).await?;
    listener.listen(channel).await?;
    tracing::info!(channel, node_id = %self_node_id, "LISTEN bridge online");

    loop {
        let notification = listener.recv().await?;
        let payload = notification.payload();

        let event: T = match serde_json::from_str(payload) {
            Ok(v) => v,
            Err(e) => {
                tracing::warn!(channel, error = %e, "dropping malformed NOTIFY payload");
                continue;
            }
        };

        // Dedupe: skip events we originated. They were already broadcast
        // in-process by publish_durable's local send.
        if let Some(origin_id) = origin(&event) {
            if origin_id == self_node_id {
                continue;
            }
        }

        // No local subscribers is fine — broadcast returns Err, drop it.
        let _ = sender.send(event);
    }
}

fn stamp_event_node_id(event: &mut ContentLakeEvent, node_id: Uuid) {
    if let ContentLakeEvent::DocumentMutation(ev) = event {
        ev.node_id = node_id;
    }
}

/// Build a size-reduced copy of an event suitable for `NOTIFY` when the full
/// payload exceeds Postgres' 8KB limit. The mutation body is preserved but
/// the potentially-large `result` document is dropped; remote SSE listeners
/// will receive the event and can re-fetch the full document by id.
fn slim_event(event: &ContentLakeEvent) -> ContentLakeEvent {
    match event {
        ContentLakeEvent::DocumentMutation(ev) => {
            ContentLakeEvent::DocumentMutation(DocumentMutationEvent {
                dataset_id: ev.dataset_id,
                document_id: ev.document_id.clone(),
                transaction_id: ev.transaction_id.clone(),
                transition: ev.transition.clone(),
                // Drop mutations AND result bodies to minimize payload; the
                // id + transition are enough for remote listeners to refetch.
                mutations: serde_json::Value::Null,
                result: None,
                node_id: ev.node_id,
            })
        }
        other => other.clone(),
    }
}

fn slim_presence(event: &PresenceEvent) -> PresenceEvent {
    match event {
        PresenceEvent::State(s) => {
            let mut slim = s.clone();
            slim.selection = None;
            PresenceEvent::State(slim)
        }
        other => other.clone(),
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

        assert!(matches!(
            rx1.recv().await.unwrap(),
            ContentLakeEvent::Reconnect
        ));
        assert!(matches!(
            rx2.recv().await.unwrap(),
            ContentLakeEvent::Reconnect
        ));
    }

    /// In-process-only construction (no pool) must still round-trip events.
    /// Guards against accidentally gating the fast path on a Postgres
    /// connection.
    #[tokio::test]
    async fn in_process_only_roundtrip_without_pool() {
        let bus = EventBus::new(8);
        let mut rx = bus.subscribe();

        let ev = ContentLakeEvent::DocumentMutation(DocumentMutationEvent {
            dataset_id: Uuid::new_v4(),
            document_id: "doc-1".to_string(),
            transaction_id: "tx-1".to_string(),
            transition: "update".to_string(),
            mutations: serde_json::json!([]),
            result: None,
            node_id: Uuid::nil(),
        });

        bus.publish(ev).unwrap();

        let received = rx.recv().await.unwrap();
        match received {
            ContentLakeEvent::DocumentMutation(m) => {
                assert_eq!(m.document_id, "doc-1");
                assert_eq!(m.transaction_id, "tx-1");
            }
            _ => panic!("expected DocumentMutation"),
        }
    }
}

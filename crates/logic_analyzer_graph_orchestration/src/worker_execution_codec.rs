use serde::Serialize;
use serde::ser::Error as _;

use logic_analyzer_graph_plan::OutputSubscriptionPlan;
use node_graph_document::{GraphState, Socket};
use platform_artifacts::{ArtifactReplicationEvent, SourceIdentity};

use crate::worker_execution::{GraphWorkerMessage, GraphWorkerRequest};

const MAGIC: &[u8; 5] = b"LGWM\x01";
const REQUEST_MAGIC: &[u8; 5] = b"LGRQ\x01";
const JSON_MESSAGE: u8 = 0;
const ARTIFACT_MESSAGE: u8 = 1;
const PUBLISHED_CHUNK: u8 = 0;
const REMOVED: u8 = 1;
const START_REQUEST: u8 = 0;
const CANCEL_REQUEST: u8 = 1;

/// Encodes one graph-worker request while keeping the graph document in its
/// native JSON representation.
///
/// `GraphState` contains a map keyed by numeric newtypes. Deserializing that
/// map through Serde's buffered internally-tagged enum representation loses
/// JSON's numeric-map-key adapter, so the transport frames the graph separately.
///
/// # Parameters
/// - `request`: Start or cancellation command to encode for a worker transport.
pub fn encode_graph_worker_request(request: &GraphWorkerRequest) -> Result<Vec<u8>, String> {
    let mut output = Vec::new();
    output.extend_from_slice(REQUEST_MAGIC);
    match request {
        GraphWorkerRequest::Start {
            sequence,
            graph,
            subscriptions,
            timeline_markers,
        } => {
            output.push(START_REQUEST);
            put_u64(&mut output, *sequence);
            put_bytes(&mut output, &encode_worker_graph(graph)?);
            put_bytes(
                &mut output,
                &serde_json::to_vec(subscriptions).map_err(|error| {
                    format!("could not encode worker output subscriptions: {error}")
                })?,
            );
            put_u32(
                &mut output,
                u32::try_from(timeline_markers.len())
                    .map_err(|_| "worker timeline-marker list is too large".to_owned())?,
            );
            for (number, timestamp_ns) in timeline_markers {
                put_u32(&mut output, *number);
                put_u64(&mut output, *timestamp_ns);
            }
        }
        GraphWorkerRequest::Cancel { sequence } => {
            output.push(CANCEL_REQUEST);
            put_u64(&mut output, *sequence);
        }
    }
    Ok(output)
}

/// Serializes a graph for execution without applying the compact saved-document
/// projection. Runtime lowering needs the definition-derived socket contract,
/// while saved documents intentionally omit that redundant presentation state.
fn encode_worker_graph(graph: &GraphState) -> Result<Vec<u8>, String> {
    let mut document = serde_json::to_value(graph)
        .map_err(|error| format!("could not encode worker graph: {error}"))?;
    let nodes = document
        .get_mut("nodes")
        .and_then(serde_json::Value::as_object_mut)
        .ok_or_else(|| "could not encode worker graph nodes".to_owned())?;
    for node in graph.nodes.values() {
        let encoded_node = nodes
            .get_mut(&node.id.0.to_string())
            .and_then(serde_json::Value::as_object_mut)
            .ok_or_else(|| "could not encode worker graph node".to_owned())?;
        encoded_node.insert(
            "inputs".to_owned(),
            serde_json::to_value(WorkerSockets(&node.inputs))
                .map_err(|error| format!("could not encode worker input sockets: {error}"))?,
        );
        encoded_node.insert(
            "outputs".to_owned(),
            serde_json::to_value(WorkerSockets(&node.outputs))
                .map_err(|error| format!("could not encode worker output sockets: {error}"))?,
        );
    }
    serde_json::to_vec(&document).map_err(|error| format!("could not encode worker graph: {error}"))
}

struct WorkerSockets<'a>(&'a [Socket]);

impl Serialize for WorkerSockets<'_> {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        self.0
            .iter()
            .map(worker_socket_value)
            .collect::<Result<Vec<_>, _>>()
            .map_err(S::Error::custom)?
            .serialize(serializer)
    }
}

fn worker_socket_value(socket: &Socket) -> Result<serde_json::Value, serde_json::Error> {
    let mut value = serde_json::to_value(socket)?;
    let object = value
        .as_object_mut()
        .expect("socket serialization produces an object");
    object.insert("name".to_owned(), serde_json::to_value(&socket.name)?);
    object.insert(
        "type_name".to_owned(),
        serde_json::to_value(&socket.type_name)?,
    );
    object.insert("color".to_owned(), serde_json::to_value(socket.color)?);
    object.insert("shape".to_owned(), serde_json::to_value(socket.shape)?);
    object.insert("allowed".to_owned(), serde_json::to_value(&socket.allowed)?);
    object.insert(
        "variadic".to_owned(),
        serde_json::to_value(&socket.variadic)?,
    );
    object.insert("visible".to_owned(), serde_json::to_value(socket.visible)?);
    object.insert(
        "editor_visible".to_owned(),
        serde_json::to_value(socket.editor_visible)?,
    );
    object.insert(
        "has_control".to_owned(),
        serde_json::to_value(socket.has_control)?,
    );
    Ok(value)
}

/// Decodes one framed graph-worker request.
pub fn decode_graph_worker_request(bytes: &[u8]) -> Result<GraphWorkerRequest, String> {
    let mut reader = Reader::new(bytes);
    if reader.take(REQUEST_MAGIC.len())? != REQUEST_MAGIC {
        return Err("graph-worker request has an invalid header".to_owned());
    }
    let request = match reader.u8()? {
        START_REQUEST => {
            let sequence = reader.u64()?;
            let graph = serde_json::from_slice(reader.bytes()?).map_err(|error| {
                format!("graph-worker request contains an invalid graph: {error}")
            })?;
            let subscriptions = serde_json::from_slice::<OutputSubscriptionPlan>(reader.bytes()?)
                .map_err(|error| {
                format!("graph-worker request contains invalid subscriptions: {error}")
            })?;
            let marker_count = reader.u32()? as usize;
            let mut timeline_markers = Vec::with_capacity(marker_count);
            for _ in 0..marker_count {
                timeline_markers.push((reader.u32()?, reader.u64()?));
            }
            GraphWorkerRequest::Start {
                sequence,
                graph,
                subscriptions,
                timeline_markers,
            }
        }
        CANCEL_REQUEST => GraphWorkerRequest::Cancel {
            sequence: reader.u64()?,
        },
        _ => return Err("graph-worker request has an unknown kind".to_owned()),
    };
    reader.finish()?;
    Ok(request)
}

/// Encodes graph-worker results without expanding artifact bytes through JSON.
///
/// # Parameters
/// - `messages`: Ordered worker messages to encode as one bounded transport frame.
pub fn encode_graph_worker_messages(messages: &[GraphWorkerMessage]) -> Result<Vec<u8>, String> {
    let mut output = Vec::new();
    output.extend_from_slice(MAGIC);
    put_u32(
        &mut output,
        u32::try_from(messages.len())
            .map_err(|_| "graph-worker message batch is too large".to_owned())?,
    );
    for message in messages {
        let mut encoded = Vec::new();
        match message {
            GraphWorkerMessage::Artifacts { sequence, events } => {
                encoded.push(ARTIFACT_MESSAGE);
                put_u64(&mut encoded, *sequence);
                put_u32(
                    &mut encoded,
                    u32::try_from(events.len())
                        .map_err(|_| "artifact event batch is too large".to_owned())?,
                );
                for event in events {
                    encode_event(&mut encoded, event)?;
                }
            }
            _ => {
                encoded.push(JSON_MESSAGE);
                encoded.extend_from_slice(
                    &serde_json::to_vec(message).map_err(|error| {
                        format!("could not encode graph-worker message: {error}")
                    })?,
                );
            }
        }
        put_u64(&mut output, encoded.len() as u64);
        output.extend_from_slice(&encoded);
    }
    Ok(output)
}

/// Decodes a compact graph-worker result batch.
pub fn decode_graph_worker_messages(bytes: &[u8]) -> Result<Vec<GraphWorkerMessage>, String> {
    let mut reader = Reader::new(bytes);
    if reader.take(MAGIC.len())? != MAGIC {
        return Err("graph-worker message batch has an invalid header".to_owned());
    }
    let count = reader.u32()? as usize;
    let mut messages = Vec::with_capacity(count);
    for _ in 0..count {
        let length = reader.length()?;
        let mut message = Reader::new(reader.take(length)?);
        match message.u8()? {
            JSON_MESSAGE => {
                messages.push(
                    serde_json::from_slice(message.remaining()).map_err(|error| {
                        format!("graph-worker message contains invalid JSON: {error}")
                    })?,
                );
                message.consume_remaining();
            }
            ARTIFACT_MESSAGE => {
                let sequence = message.u64()?;
                let count = message.u32()? as usize;
                let mut events = Vec::with_capacity(count);
                for _ in 0..count {
                    events.push(decode_event(&mut message)?);
                }
                messages.push(GraphWorkerMessage::Artifacts { sequence, events });
            }
            _ => return Err("graph-worker message has an unknown encoding".to_owned()),
        }
        message.finish()?;
    }
    reader.finish()?;
    Ok(messages)
}

fn encode_event(output: &mut Vec<u8>, event: &ArtifactReplicationEvent) -> Result<(), String> {
    match event {
        ArtifactReplicationEvent::PublishedChunk {
            namespace,
            identity,
            offset,
            total_length,
            data,
            complete,
        } => {
            output.push(PUBLISHED_CHUNK);
            put_string(output, namespace)?;
            output.extend_from_slice(identity.as_bytes());
            put_u64(output, *offset);
            put_u64(output, *total_length);
            output.push(u8::from(*complete));
            put_u64(output, data.len() as u64);
            output.extend_from_slice(data);
        }
        ArtifactReplicationEvent::Removed {
            namespace,
            identity,
        } => {
            output.push(REMOVED);
            put_string(output, namespace)?;
            output.extend_from_slice(identity.as_bytes());
        }
    }
    Ok(())
}

fn decode_event(reader: &mut Reader<'_>) -> Result<ArtifactReplicationEvent, String> {
    match reader.u8()? {
        PUBLISHED_CHUNK => {
            let namespace = reader.string()?;
            let identity = SourceIdentity::from_bytes(reader.take(32)?.try_into().unwrap());
            let offset = reader.u64()?;
            let total_length = reader.u64()?;
            let complete = match reader.u8()? {
                0 => false,
                1 => true,
                _ => return Err("artifact completion flag is invalid".to_owned()),
            };
            let data_length = reader.length()?;
            let data = reader.take(data_length)?.to_vec();
            Ok(ArtifactReplicationEvent::PublishedChunk {
                namespace,
                identity,
                offset,
                total_length,
                data,
                complete,
            })
        }
        REMOVED => Ok(ArtifactReplicationEvent::Removed {
            namespace: reader.string()?,
            identity: SourceIdentity::from_bytes(reader.take(32)?.try_into().unwrap()),
        }),
        _ => Err("artifact replication event has an unknown encoding".to_owned()),
    }
}

fn put_string(output: &mut Vec<u8>, value: &str) -> Result<(), String> {
    put_u32(
        output,
        u32::try_from(value.len()).map_err(|_| "artifact namespace is too large".to_owned())?,
    );
    output.extend_from_slice(value.as_bytes());
    Ok(())
}

fn put_u32(output: &mut Vec<u8>, value: u32) {
    output.extend_from_slice(&value.to_le_bytes());
}

fn put_u64(output: &mut Vec<u8>, value: u64) {
    output.extend_from_slice(&value.to_le_bytes());
}

fn put_bytes(output: &mut Vec<u8>, value: &[u8]) {
    put_u64(output, value.len() as u64);
    output.extend_from_slice(value);
}

struct Reader<'a> {
    bytes: &'a [u8],
    cursor: usize,
}

impl<'a> Reader<'a> {
    fn new(bytes: &'a [u8]) -> Self {
        Self { bytes, cursor: 0 }
    }

    fn u8(&mut self) -> Result<u8, String> {
        Ok(self.take(1)?[0])
    }

    fn u32(&mut self) -> Result<u32, String> {
        Ok(u32::from_le_bytes(self.take(4)?.try_into().unwrap()))
    }

    fn u64(&mut self) -> Result<u64, String> {
        Ok(u64::from_le_bytes(self.take(8)?.try_into().unwrap()))
    }

    fn length(&mut self) -> Result<usize, String> {
        usize::try_from(self.u64()?)
            .map_err(|_| "graph-worker field length exceeds this host".to_owned())
    }

    fn string(&mut self) -> Result<String, String> {
        let length = self.u32()? as usize;
        std::str::from_utf8(self.take(length)?)
            .map(str::to_owned)
            .map_err(|_| "artifact namespace is not UTF-8".to_owned())
    }

    fn bytes(&mut self) -> Result<&'a [u8], String> {
        let length = self.length()?;
        self.take(length)
    }

    fn take(&mut self, length: usize) -> Result<&'a [u8], String> {
        let end = self
            .cursor
            .checked_add(length)
            .ok_or_else(|| "graph-worker message length overflow".to_owned())?;
        let value = self
            .bytes
            .get(self.cursor..end)
            .ok_or_else(|| "graph-worker message is truncated".to_owned())?;
        self.cursor = end;
        Ok(value)
    }

    fn remaining(&self) -> &'a [u8] {
        &self.bytes[self.cursor..]
    }

    fn consume_remaining(&mut self) {
        self.cursor = self.bytes.len();
    }

    fn finish(&self) -> Result<(), String> {
        if self.cursor == self.bytes.len() {
            Ok(())
        } else {
            Err("graph-worker message has trailing bytes".to_owned())
        }
    }
}

#[cfg(test)]
mod worker_execution_codec_tests {
    use node_graph_document::NodeId;

    use super::*;
    use crate::worker_execution::GraphWorkerFailure;

    #[test]
    fn requests_round_trip_nonempty_saved_graphs() {
        let graph: node_graph_document::GraphState = serde_json::from_str(
            r#"{
                "nodes": {
                    "0": {
                        "id": 0,
                        "kind": "Regular",
                        "title": "Fixture",
                        "type_name": "Fixture",
                        "header_color": [80, 80, 80, 255],
                        "pos": { "x": 0.0, "y": 0.0 },
                        "inputs": [],
                        "outputs": [],
                        "collapsed": false,
                        "muted": false,
                        "state": null,
                        "selected": false
                    }
                },
                "connections": [],
                "frames": [],
                "next_id": 1,
                "next_frame_id": 0
            }"#,
        )
        .unwrap();
        let expected_snapshot = graph.semantic_snapshot();
        let request = GraphWorkerRequest::Start {
            sequence: 11,
            graph,
            subscriptions: OutputSubscriptionPlan::new(),
            timeline_markers: vec![(2, 45)],
        };

        let decoded =
            decode_graph_worker_request(&encode_graph_worker_request(&request).unwrap()).unwrap();

        let GraphWorkerRequest::Start {
            sequence,
            graph,
            timeline_markers,
            ..
        } = decoded
        else {
            panic!("start request changed kind");
        };
        assert_eq!(sequence, 11);
        assert_eq!(graph.semantic_snapshot(), expected_snapshot);
        assert_eq!(timeline_markers, [(2, 45)]);
    }

    #[test]
    fn artifact_bytes_round_trip_without_json_expansion() {
        let data = vec![0xa5; 1024 * 1024];
        let messages = vec![
            GraphWorkerMessage::Started { sequence: 3 },
            GraphWorkerMessage::Progress {
                sequence: 3,
                nodes: vec![(NodeId(7), 42)],
            },
            GraphWorkerMessage::Failed {
                sequence: 3,
                error: GraphWorkerFailure::Artifact("replication stopped".into()),
            },
            GraphWorkerMessage::Artifacts {
                sequence: 3,
                events: vec![ArtifactReplicationEvent::PublishedChunk {
                    namespace: "derived-word-blocks-v1-test".to_owned(),
                    identity: SourceIdentity::from_bytes([9; 32]),
                    offset: 0,
                    total_length: data.len() as u64,
                    data: data.clone(),
                    complete: true,
                }],
            },
        ];

        let encoded = encode_graph_worker_messages(&messages).unwrap();
        assert!(encoded.len() < data.len() + 512);
        assert_eq!(decode_graph_worker_messages(&encoded).unwrap(), messages);
    }
}

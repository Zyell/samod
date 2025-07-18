use std::collections::{HashMap, VecDeque};

use samod_core::{
    ConnectionId, StorageKey,
    actors::hub::{DispatchedCommand, HubEvent},
};

mod doc_actor_runner;
mod running_doc_ids;
pub use running_doc_ids::RunningDocIds;
mod samod_id;
use samod_id::SamodId;
mod samod_ref;
mod samod_wrapper;
pub use samod_ref::SamodRef;
pub use samod_wrapper::SamodWrapper;
mod storage;
pub(crate) use storage::Storage;

pub struct Network {
    samods: HashMap<SamodId, SamodWrapper>,
    connections: Vec<Connection>,
}

struct Connection {
    left_connection: ConnectionId,
    left_samod: SamodId,
    right_connection: ConnectionId,
    right_samod: SamodId,
}

impl Default for Network {
    fn default() -> Self {
        Self::new()
    }
}

impl Network {
    pub fn new() -> Self {
        Network {
            samods: HashMap::new(),
            connections: Vec::new(),
        }
    }

    pub fn create_samod<S: AsRef<str>>(&mut self, nickname: S) -> SamodId {
        let samod = SamodWrapper::new(nickname.as_ref().to_string());
        let id = SamodId::new();
        self.samods.insert(id, samod);
        self.run_until_quiescent();
        id
    }

    pub fn create_samod_with_storage<S: AsRef<str>>(
        &mut self,
        nickname: S,
        storage: HashMap<StorageKey, Vec<u8>>,
    ) -> SamodId {
        let samod =
            SamodWrapper::new_with_storage(nickname.as_ref().to_string(), Storage::from(storage));
        let id = SamodId::new();
        self.samods.insert(id, samod);
        self.run_until_quiescent();
        id
    }

    pub fn connect(&mut self, left: SamodId, right: SamodId) {
        // Left samod initiates with an outgoing connection
        let left_connection = self
            .samods
            .get_mut(&left)
            .expect("Left Samod not found")
            .create_connection();
        // Right samod receives with an incoming connection
        let right_connection = self
            .samods
            .get_mut(&right)
            .expect("Right Samod not found")
            .create_incoming_connection();

        self.connections.push(Connection {
            left_connection,
            left_samod: left,
            right_connection,
            right_samod: right,
        });
    }

    pub fn disconnect(&mut self, left: SamodId, right: SamodId) {
        let (left_conn_id, right_conn_id) = match self.connections.iter().find_map(|c| {
            // Make sure that we get the connection IDs the right way around
            // even if the order they were passed to us was reversed (i.e. the
            // disconnect arguments were the opposite way around to the connect
            // arguments)
            if c.left_samod == left && c.right_samod == right {
                Some((c.left_connection, c.right_connection))
            } else if c.right_samod == left && c.left_samod == right {
                Some((c.right_connection, c.left_connection))
            } else {
                None
            }
        }) {
            Some(ids) => ids,
            None => return,
        };

        // Remove the connection from the network
        self.connections
            .retain(|c| c.left_samod != left && c.right_samod != right);

        let left_evt = HubEvent::connection_lost(left_conn_id);
        self.samods
            .get_mut(&left)
            .unwrap()
            .inbox
            .push_back(left_evt);
        let right_evt = HubEvent::connection_lost(right_conn_id);
        self.samods
            .get_mut(&right)
            .unwrap()
            .inbox
            .push_back(right_evt);
        self.run_until_quiescent();
    }

    pub fn run_until_quiescent(&mut self) {
        loop {
            let mut msgs_this_round: HashMap<SamodId, VecDeque<HubEvent>> = HashMap::new();

            // For each samod, handle it's events
            for samod in self.samods.values_mut() {
                samod.handle_events();

                for (connection_id, msgs) in samod.outbox.drain() {
                    if let Some(connection) = self.connections.iter().find(|c| {
                        c.left_connection == connection_id || c.right_connection == connection_id
                    }) {
                        let (target_samod_id, target_connection_id) =
                            if connection.left_connection == connection_id {
                                (connection.right_samod, connection.right_connection)
                            } else {
                                (connection.left_samod, connection.left_connection)
                            };

                        for msg in msgs {
                            let DispatchedCommand { event, .. } =
                                HubEvent::receive(target_connection_id, msg);
                            msgs_this_round
                                .entry(target_samod_id)
                                .or_default()
                                .push_back(event);
                        }
                    }
                }
            }
            let quiet = msgs_this_round.values().all(|m| m.is_empty());
            if quiet {
                break;
            }
            for (target_samod_id, events) in msgs_this_round {
                if let Some(samod) = self.samods.get_mut(&target_samod_id) {
                    samod.inbox.extend(events);
                } else {
                    panic!("Target Samod not found: {target_samod_id:?}");
                }
            }
        }
    }

    pub fn samod<'a>(&'a mut self, id: &'a SamodId) -> SamodRef<'a> {
        SamodRef {
            network: self,
            samod_id: id,
        }
    }
}

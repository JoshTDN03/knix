#[allow(dead_code,unused,unused_must_use)]
use crate::utils::create_delay;
use crate::utils::send_post_json_message;
use crate::utils::WorkflowInfo;
use crate::CommandResponseChannel;
use crate::TriggerCommand;
use crate::TriggerCommandChannel;
use serde::{Deserialize, Serialize};
use crate::TriggerManager;
use json::JsonValue;
use lapin::{
    options::*, types::FieldTable, Channel, Connection, ConnectionProperties, Consumer, Queue,
    Result,
};
use log::*;
use std::borrow::Borrow;
use std::error::Error;
use std::rc::Rc;
use tokio::stream::StreamExt;
use tokio::sync::mpsc::{channel, Receiver, Sender};
use tokio::sync::oneshot;

use crate::trigger_manager::send_status_update_from_trigger_to_manager;
use crate::trigger_manager::TriggerManagerCommandChannelSender;
use crate::utils::find_element_index;
use crate::utils::TriggerError;
use crate::utils::TriggerWorkflowMessage;
use crate::TriggerStatus;

#[derive(Clone, Debug, Serialize)]
pub struct AMQPSubscriberInfo {
    amqp_addr: String,
    routing_key: String,
    exchange: String,  // "egress_exchange", by default
    durable: bool,     // False, by default
    exclusive: bool,   // False, by default
    auto_delete: bool, // True by default
    no_wait: bool,     // True by default 
    with_ack: bool,     // False, by default
                        // False means no manual ack, hence means auto_ack
                        // This is converted to basic_consume option: no_ack = !with_ack
                        
}

pub struct AMQPTrigger {
    trigger_id: String,
    trigger_name: String,
    amqp_sub_info: AMQPSubscriberInfo,
    workflows: Vec<WorkflowInfo>,
    // Sender return
    cmd_channel_tx: TriggerCommandChannel,
}

#[derive(Clone, Debug, Serialize)]
pub struct AMQPTriggerStatus {
    trigger_name: String,
    trigger_status: String,
    trigger_type: String,
    trigger_id: String,
    status_msg: String,
    trigger_count: u64,
    associated_workflows: Vec<WorkflowInfo>,
    trigger_info: AMQPSubscriberInfo,
}

impl AMQPTrigger {
    pub fn spawn(
        trigger_id: &String,
        trigger_name: &String,
        amqp_sub_info: AMQPSubscriberInfo,
        workflows: Vec<WorkflowInfo>,
        manager_cmd_channel_tx: TriggerManagerCommandChannelSender,
    ) -> std::result::Result<AMQPTrigger, ()> {
        let (cmd_channel_tx, cmd_channel_rx) =
            channel::<(TriggerCommand, CommandResponseChannel)>(5);
        tokio::spawn(amqp_actor_retry_loop(
            trigger_id.clone(),
            trigger_name.clone(),
            amqp_sub_info.clone(),
            workflows.clone(),
            cmd_channel_rx,
            manager_cmd_channel_tx,
        ));
        Ok(AMQPTrigger {
            trigger_id: trigger_id.clone(),
            trigger_name: trigger_name.clone(),
            amqp_sub_info: amqp_sub_info.clone(),
            workflows: workflows.clone(),
            cmd_channel_tx,
        })
    }

    // pub async fn get_status(&mut self) -> std::result::Result<String, ()> {
    //     let mut response = Err(());
    //     let (resp_tx, resp_rx) = oneshot::channel();
    //     let send_result = self
    //         .cmd_channel_tx
    //         .send((TriggerCommand::Status, resp_tx))
    //         .await;
    //     if let Ok(m) = send_result {
    //         response = match resp_rx.await {
    //             Ok(msg) => {
    //                 debug!("[get_status] {} status response: {}", &self.trigger_id, msg);
    //                 Ok(msg)
    //             }
    //             _ => {
    //                 info!("[get_status] {} Error status response", &self.trigger_id);
    //                 Err(())
    //             }
    //         };
    //     }
    //     response
    // }

    // pub async fn stop(&mut self) -> std::result::Result<String, ()> {
    //     let mut response = Err(());
    //     let (resp_tx, resp_rx) = oneshot::channel();
    //     let send_result = self
    //         .cmd_channel_tx
    //         .send((TriggerCommand::Stop, resp_tx))
    //         .await;
    //     if let Ok(m) = send_result {
    //         response = match resp_rx.await {
    //             Ok(msg) => {
    //                 debug!("[stop] {} stop response: {}", &self.trigger_id, msg);
    //                 Ok(msg)
    //             }
    //             _ => {
    //                 info!("[stop] {} Error stop response", &self.trigger_id);
    //                 Err(())
    //             }
    //         };
    //     }
    //     response
    // }

    // pub async fn is_alive(&mut self) -> bool {
    //     if let Ok(msg) = self.get_status().await {
    //         return true;
    //     } else {
    //         return false;
    //     }
    // }
}

pub async fn handle_create_amqp_trigger(
    trigger_id: &String,
    trigger_name: &String,
    workflows: Vec<WorkflowInfo>,
    request_body: &String,
    manager_cmd_channel_tx: TriggerManagerCommandChannelSender,
) -> std::result::Result<TriggerCommandChannel, String> {
    let json_body = json::parse(request_body).unwrap();
    let trigger_info = &json_body["trigger_info"];

    if !(trigger_info.has_key("amqp_addr") && trigger_info.has_key("routing_key")) {
        return Err("One of the required fields, 'amqp_addr' or 'routing_key' is missing".into());
    }

    let amqp_sub_info = AMQPSubscriberInfo {
        amqp_addr: trigger_info["amqp_addr"].to_string(),
        routing_key: trigger_info["routing_key"].to_string(),
        exchange: if trigger_info.has_key("exchange") {
            trigger_info["exchange"].to_string()
        } else {
            "egress_exchange".into()
        },
        durable: if trigger_info.has_key("durable") {
            trigger_info["durable"].as_bool().unwrap()
        } else {
            false
        },
        exclusive: if trigger_info.has_key("exclusive") {
            trigger_info["exclusive"].as_bool().unwrap()
        } else {
            false
        },
        auto_delete: if trigger_info.has_key("auto_delete") {
            trigger_info["auto_delete"].as_bool().unwrap()
        } else {
            true
        },
        no_wait: if trigger_info.has_key("no_wait") {
            trigger_info["no_wait"].as_bool().unwrap()
        } else {
            true
        },
        with_ack: if trigger_info.has_key("with_ack") {
            trigger_info["with_ack"].as_bool().unwrap()
        } else {
            false
        },
    };

    let amqp_trigger = AMQPTrigger::spawn(
        &trigger_id,
        &trigger_name, 
        amqp_sub_info,
        workflows,
        manager_cmd_channel_tx,
    )
    .unwrap();

    Ok(amqp_trigger.cmd_channel_tx)
}


async fn send_amqp_data(
    workflows: Vec<WorkflowInfo>,
    amqp_data: std::vec::Vec<u8>,
    trigger_id: String,
    trigger_name: String,
    source: String,
) {
    let workflow_msg: TriggerWorkflowMessage;
    match String::from_utf8(amqp_data) {
        Ok(v) => {
            debug!("[send_amqp_data] source: {}, data: {}", &source, &v);
            for workflow_info in workflows {
                let workflow_msg = TriggerWorkflowMessage {
                    trigger_status: "ready".into(),
                    trigger_type: "amqp".into(),
                    trigger_name: trigger_name.clone(),
                    workflow_name: workflow_info.workflow_name,
                    source: source.clone(),
                    data: v.clone(), // TODO: Figure out how to pass the String around,
                                     // without copying and keeping the borrow checker happy!
                };
                let serialized_workflow_msg = serde_json::to_string(&workflow_msg);
                debug!(
                    "[send_amqp_data] Trigger id {}, Sending message: {}",
                    trigger_id,
                    serialized_workflow_msg.as_ref().unwrap()
                );
                send_post_json_message(
                    workflow_info.workflow_url,
                    serialized_workflow_msg.unwrap(),
                    "".into(),
                    workflow_info.workflow_state.clone(),
                    true,
                ).await;
            }
        }
        Err(e) => {
            let data_str = format!(
                "Trigger id {}, Message received with Invalid UTF-8 sequence: {}",
                &trigger_id, e
            );
            warn!("[send_amqp_data] {}", &data_str);
        }
    };
}

pub async fn amqp_actor_retry_loop(
    trigger_id: String,
    trigger_name: String,
    amqp_sub_info: AMQPSubscriberInfo,
    workflows: Vec<WorkflowInfo>,
    mut cmd_channel_rx: Receiver<(TriggerCommand, CommandResponseChannel)>,
    mut manager_cmd_channel_tx: TriggerManagerCommandChannelSender,
) {
    let res: std::result::Result<(), Box<dyn std::error::Error + Send + Sync>> = amqp_actor_loop(
        &trigger_id,
        &trigger_name,
        &amqp_sub_info,
        workflows,
        &mut cmd_channel_rx,
        &mut manager_cmd_channel_tx,
    )
    .await;
    match res {
        Ok(()) => {
            info!(
                "[amqp_actor_retry_loop] {} amqp_actor_loop finished without errors",
                trigger_id.clone()
            );
        }
        Err(e) => {
            warn!(
                "[amqp_actor_retry_loop] {} amqp_actor_loop finished with an error: {}",
                trigger_id.clone(),
                e.to_string()
            );
            send_status_update_from_trigger_to_manager(
                trigger_id.clone(),
                TriggerStatus::StoppedError,
                format!("Error: {}", e.to_string()),
                manager_cmd_channel_tx.clone(),
            )
            .await;
        }
    }
}

pub async fn amqp_actor_loop(
    trigger_id: &String,
    trigger_name: &String,
    amqp_sub_info: &AMQPSubscriberInfo,
    mut workflows: Vec<WorkflowInfo>,
    cmd_channel_rx: &mut Receiver<(TriggerCommand, CommandResponseChannel)>,
    manager_cmd_channel_tx: &mut TriggerManagerCommandChannelSender,
) -> std::result::Result<(), Box<dyn std::error::Error + Send + Sync>> {
    info!("[amqp_actor_loop] {} start", trigger_id);
    
    let mut trigger_count: u64 = 0;

    let addr = &amqp_sub_info.amqp_addr;
    info!("[amqp_actor_loop] {} Before [connect], addr: {}", trigger_id, addr);
    let conn: lapin::Connection =
        Connection::connect(addr, ConnectionProperties::default()).await?;
    info!("[amqp_actor_loop] {} After [connect]", trigger_id);

    //receive channel
    let channel: Channel = conn.create_channel().await?;
    info!(
        "[amqp_actor_loop] {} After [create_channel] state: {:?}",
        trigger_id,
        conn.status().state()
    );

    /*
    let eops = ExchangeDeclareOptions {
        durable: amqp_sub_info.durable,
        ..ExchangeDeclareOptions::default()
    };
    let exchange = channel
        .exchange_declare(
            &amqp_sub_info.exchange,
            lapin::ExchangeKind::Topic,
            eops,
            FieldTable::default(),
        )
        .await?;
    info!("[amqp_actor_loop] {} After exchange_declare", trigger_id);
    */

    let qops = QueueDeclareOptions {
        durable: amqp_sub_info.durable,
        exclusive: amqp_sub_info.exclusive,
        auto_delete: amqp_sub_info.auto_delete,
        nowait: amqp_sub_info.no_wait,
        ..QueueDeclareOptions::default()
    };
    info!(
        "[amqp_actor_loop] {} Before [queue_declare], queue_name: {}, options {:?}",
        trigger_id, trigger_id, qops
    );

    let queue: lapin::Queue = channel
        .queue_declare(trigger_id.as_str(), qops, FieldTable::default())
        .await?;

    info!(
        "[amqp_actor_loop] {} After [queue_declare], queue_name: {}, options {:?}",
        trigger_id, trigger_id, qops
    );

    let qbops = QueueBindOptions {
        //nowait: amqp_sub_info.no_wait,
        ..QueueBindOptions::default()
    };

    info!("[amqp_actor_loop] {} Before [queue_bind], queue_name: {}, exchange: {}, routing_key: {}, options: {:?}", trigger_id, trigger_id, &amqp_sub_info.exchange, &amqp_sub_info.routing_key, qbops);

    let qbind_response = channel
        .queue_bind(
            trigger_id.as_str(),
            &amqp_sub_info.exchange,
            &amqp_sub_info.routing_key,
            qbops,
            FieldTable::default(),
        )
        .await?;

    info!("[amqp_actor_loop] {} After [queue_bind], queue_name: {}, exchange: {}, routing_key: {}, options: {:?}", trigger_id, trigger_id, &amqp_sub_info.exchange, &amqp_sub_info.routing_key, qbops);

    let cops = BasicConsumeOptions {
        no_ack: !amqp_sub_info.with_ack,
        ..BasicConsumeOptions::default()
    };
    info!("[amqp_actor_loop] {} Before [basic_consume], queue_name: {}, options: {:?}", trigger_id, trigger_id, cops);
    let mut consumer: Consumer = channel
        .basic_consume(trigger_id.as_str(), "", cops, FieldTable::default())
        .await?;

    info!("[amqp_actor_loop] {} After [basic_consume], queue_name: {}, options: {:?}", trigger_id, trigger_id, cops);

    info!("[amqp_actor_loop] {} Ready to consume", trigger_id);
    send_status_update_from_trigger_to_manager(
        trigger_id.clone(),
        TriggerStatus::Ready,
        "".into(),
        manager_cmd_channel_tx.clone(),
    )
    .await;

    loop {
        tokio::select! {
            cmd = cmd_channel_rx.recv() => {
                match cmd {
                    Some((c, resp)) => {
                        match c {
                            TriggerCommand::GetStatus => {
                                info!("[amqp_actor_loop] {} Status cmd recv", trigger_id);
                                let status_info = AMQPTriggerStatus {
                                    trigger_status: "ready".into(),
                                    trigger_type: "amqp".into(),
                                    trigger_name: trigger_name.clone(),
                                    trigger_id: trigger_id.clone(),
                                    status_msg: "".into(),
                                    trigger_count,
                                    associated_workflows: workflows.clone(),
                                    trigger_info: amqp_sub_info.clone(),
                                };
                                let serialized_status_msg: String = serde_json::to_string(&status_info).unwrap();

                                resp.send((true, serialized_status_msg));
                            }
                            TriggerCommand::AddWorkflows(workflows_to_add) => {
                                for workflow in workflows_to_add.clone() {
                                    let idx = find_element_index(&workflow, &workflows);
                                    if idx >= 0 {
                                        workflows.remove(idx as usize);
                                    }
                                }

                                for workflow in workflows_to_add {
                                    workflows.push(workflow.clone());
                                }
                                resp.send((true, "ok".to_string()));
                            }
                            TriggerCommand::RemoveWorkflows(workflows_to_remove) => {
                                for workflow in workflows_to_remove {
                                    let idx = find_element_index(&workflow, &workflows);
                                    if idx >= 0 {
                                        workflows.remove(idx as usize);
                                    }
                                }
                                resp.send((true, "ok".to_string()));
                            }
                            TriggerCommand::Stop => {
                                info!("[amqp_actor_loop] {} Stop cmd recv", trigger_id);
                                resp.send((true, "ok".to_string()));
                                conn.close(0, "closing").await;
                                send_status_update_from_trigger_to_manager(
                                    trigger_id.clone(),
                                    TriggerStatus::StoppedNormal,
                                    "".into(),
                                    manager_cmd_channel_tx.clone(),
                                ).await;
                                break;
                            }
                        }
                    }
                    None => {
                        let ret_msg = format!("[amqp_actor_loop] Trigger id {}, None recv on command channel", trigger_id);
                        warn!("{}", ret_msg);
                        return Err(Box::new(TriggerError{ err_msg: ret_msg.clone()}));
                    },
                }
            }
            msg = consumer.next() => {
                match msg {
                    Some(delivery) => {
                        match delivery {
                            Ok((chan, amqp_msg)) => {
                                trigger_count += 1;
                                if workflows.len() > 0 {
                                    let actual_routing_key: String = amqp_msg.routing_key.as_str().to_string() ;
                                    //tokio::spawn(send_amqp_data(workflows.clone(), amqp_msg.data, trigger_id.clone(), trigger_name.clone(), actual_routing_key)).await;
                                    send_amqp_data(workflows.clone(), amqp_msg.data, trigger_id.clone(), trigger_name.clone(), actual_routing_key).await; 
                                }
                            }
                            Err(e) => {
                                let ret_msg = format!("[amqp_actor_loop] Trigger id {}, recv a msg on amqp channel, but unwrapping produced an error: {:?}", trigger_id, e);
                                warn!("{}", ret_msg);
                                return Err(Box::new(TriggerError{ err_msg: ret_msg.clone()}));
                            }
                        }
                    }
                    None => {
                        let ret_msg = format!("[amqp_actor_loop] Trigger id {}, None recv on amqp channel. Probably closed", trigger_id);
                        warn!("{}", ret_msg);
                        return Err(Box::new(TriggerError{ err_msg: ret_msg.clone()}));
                    }
                }
            }
        }
    }
    Ok(())
}

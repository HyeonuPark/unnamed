use std::convert::Into;
use std::sync::mpsc;
use std::thread;
use std::collections::HashMap;
use std::fmt::Write;
use std::sync::mpsc::Sender;

use num_cpus;

use kernel::Kernel;
use task::Task;
use worker::{self, WorkerId};
use event::Event;
use board::Board;

use event::CoreCommand::{Publish, Listen, Ignore};

pub struct Flock<K: Kernel> {
    kernel: K,
    worker_count: usize,
    prefix: String,
}

impl<K: Kernel + 'static> Flock<K> {
    pub fn new(kernel: K) -> Self {
        Flock {
            kernel: kernel,
            worker_count: num_cpus::get(),
            prefix: "flock-".into(),
        }
    }

    pub fn num_workers(mut self, worker_count: usize) -> Self {
        self.worker_count = worker_count;
        self
    }

    pub fn name_prefix<T: Into<String>>(mut self, prefix: T) -> Self {
        self.prefix = prefix.into();
        self
    }

    pub fn run(self, task: Box<Task<Kernel = K>>) {
        assert!(self.worker_count > 0);

        let kernel = &self.kernel;

        let mut board: Board<_, _, Sender<_>> = Board::new();
        let channels = (0..self.worker_count).map(|_| mpsc::channel());

        let mut listeners = HashMap::new();
        let mut threads = Vec::new();

        for (index, (sender, receiver)) in channels.enumerate() {
            let id = WorkerId(index);

            let mut thread_name = String::new();
            write!(thread_name, "{}{}", self.prefix, index)
                .expect("Failed to construct worker thread's name");

            let thread_handle = {
                let id = id.clone();
                let sink = kernel.create_sink();

                thread::Builder::new()
                    .name(thread_name)
                    .spawn(move || { worker::run::<K>(id, receiver, sink); })
                    .expect("Failed to spawn worker thread")
            };

            threads.push(thread_handle);
            listeners.insert(id, sender);
        }

        let (listeners, threads) = (listeners, threads);

        // TODO: pass initial task to some worker to execute it

        let event_id_state = &mut 0;

        kernel.run(|command| {
            match command {
                Publish(topic, data, is_last) => {
                    if let Some(senders) = board.query(&topic) {
                        for sender in senders {
                            sender.send(Event::new(topic.clone(), data.clone(),
                                is_last, event_id_state));
                        }
                    }
                }
                Listen(wid, topic) => {
                    board.subscribe(topic, wid.clone(), listeners[&wid].clone());
                }
                Ignore(wid, topic) => {
                    board.unsubscribe(topic, wid);
                }
            };
        });

        ::std::mem::drop(listeners);

        for handle in threads {
            handle.join().unwrap();
        }
    }
}

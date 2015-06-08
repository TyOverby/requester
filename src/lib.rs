extern crate clock_ticks;

use std::sync::mpsc::{channel, Sender, Receiver};
use std::sync::mpsc::TryRecvError::*;
use std::thread;

use clock_ticks::precise_time_ms;
use std::boxed::Box;

type FilterFn<T> = Box<Fn(&T) -> bool + Send>;

pub struct Requester<T: Send> {
    subscribe_no_timeout: Sender<(FilterFn<T>, Sender<T>)>,
    subscribe_timeout: Sender<(u64, FilterFn<T>, Sender<Option<T>>)>
}

impl <T: Clone + Send + 'static> Requester<T> {
    pub fn new(source: Receiver<T>) -> Requester<T> {
        let mut without_timeouts: Vec<(FilterFn<T>, Sender<T>)> = Vec::new();
        let mut with_timeouts: Vec<(u64, FilterFn<T>, Sender<Option<T>>)> = Vec::new();

        let (nt_s, nt_r) = channel();
        let (t_s, t_r) = channel();

        let mut no_more_subscribers = false;

        thread::spawn(move || {
            loop {
                // Get new subscribers (no timeout)
                loop {
                    match nt_r.try_recv() {
                        Ok(sub) => without_timeouts.push(sub),
                        Err(Empty) => break,
                        Err(Disconnected) => {
                            no_more_subscribers = true;
                            break;
                        }
                    }
                }

                // Get new subscribers (timeout)
                loop {
                    match t_r.try_recv() {
                        Ok(sub) => with_timeouts.push(sub),
                        Err(Empty) => break,
                        Err(Disconnected) => {
                            no_more_subscribers = true;
                            break;
                        }
                    }
                }

                // Get rid of timed out receivers
                let current = precise_time_ms();
                with_timeouts.retain(|&(timeout, _, ref sender)| {
                    if timeout < current {
                        let _ = sender.send(None);
                        false
                    } else {
                        true
                    }
                });

                // Get a value and send it out to all the listeners
                match source.try_recv() {
                    Ok(value) => {
                        without_timeouts.retain(|&(ref filter, ref sender)| {
                            if filter(&value) {
                                let res = sender.send(value.clone());
                                res.is_ok()
                            } else {
                                true
                            }
                        });

                        with_timeouts.retain(|&(_, ref filter, ref sender)| {
                            if filter(&value) {
                                let res = sender.send(Some(value.clone()));
                                res.is_ok()
                            } else {
                                true
                            }
                        });
                    }
                    Err(Empty) => { },
                    Err(Disconnected) => return
                }

                if no_more_subscribers && without_timeouts.is_empty() && with_timeouts.is_empty() {
                    return;
                }
            }
        });

        Requester {
            subscribe_no_timeout: nt_s,
            subscribe_timeout: t_s
        }
    }

    pub fn request<F>(&self, predicate: F) -> Receiver<T> where F: Fn(&T) -> bool + Send + 'static{
        let boxed = Box::new(predicate) as FilterFn<T>;
        let (sx, rx) = channel();
        self.subscribe_no_timeout.send((boxed, sx)).unwrap();
        rx
    }

    pub fn request_timeout<F>(&self, timeout_ms: u64, predicate: F) -> Receiver<Option<T>> where F: Fn(&T) -> bool + Send + 'static {
        let boxed = Box::new(predicate) as FilterFn<T>;
        let (sx, rx) = channel();
        self.subscribe_timeout.send((timeout_ms, boxed, sx)).unwrap();
        rx
    }
}

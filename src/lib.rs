extern crate clock_ticks;

use std::sync::mpsc::{channel, Sender, Receiver};
use std::sync::mpsc::TryRecvError::*;
use std::thread;

use clock_ticks::precise_time_ms;
use std::boxed::Box;

type FilterFn<T> = Box<Fn(&T) -> bool + Send>;

#[derive(Clone)]
pub struct Requester<T: Send> {
    subscribe_no_timeout: Sender<(FilterFn<T>, Sender<T>)>,
    subscribe_timeout: Sender<(u64, FilterFn<T>, Sender<T>)>
}

impl <T: Clone + Send + 'static> Requester<T> {
    /// Create a new Requester from a source of items.
    ///
    /// This function returns a Requester that can be queried for values,
    /// as well as a Receiver<T> that will have all the original values moved into
    /// without any clones.
    pub fn new(source: Receiver<T>) -> (Requester<T>, Receiver<T>) {
        let mut without_timeouts: Vec<(FilterFn<T>, Sender<T>)> = Vec::new();
        let mut with_timeouts: Vec<(u64, FilterFn<T>, Sender<T>)> = Vec::new();

        let (nt_s, nt_r) = channel();
        let (t_s, t_r) = channel();

        let (forward_s, forward_r) = channel();
        let mut forward_s = Some(forward_s);

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
                with_timeouts.retain(|&(deadline, _, _)| deadline >= current);

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
                                let res = sender.send(value.clone());
                                res.is_ok()
                            } else {
                                true
                            }
                        });

                        forward_s = if let Some(forward) = forward_s {
                            if forward.send(value).is_err() {
                                None
                            } else {
                                Some(forward)
                            }
                        } else {
                            None
                        };
                    }
                    Err(Empty) => thread::yield_now(),
                    Err(Disconnected) => return
                }

                if no_more_subscribers &&
                   without_timeouts.is_empty() &&
                   with_timeouts.is_empty() &&
                   forward_s.is_none() {
                    return;
                }
            }
        });

        (Requester {
            subscribe_no_timeout: nt_s,
            subscribe_timeout: t_s
        }, forward_r)
    }

    /// Returns a Receiver<T> where for each element, `predicate(t) == true`.
    pub fn request<F>(&self, predicate: F) -> Receiver<T> where F: Fn(&T) -> bool + Send + 'static{
        let boxed = Box::new(predicate) as FilterFn<T>;
        let (sx, rx) = channel();
        self.subscribe_no_timeout.send((boxed, sx)).unwrap();
        rx
    }

    /// Returns a Receiver<T> where for each element, `predicate(t) == true`.
    ///
    /// The sending end of the channel will be closed when the timeout is reached.
    pub fn request_timeout<F>(&self, timeout_ms: u64, predicate: F) -> Receiver<T> where F: Fn(&T) -> bool + Send + 'static {
        let deadline = precise_time_ms() + timeout_ms;
        let boxed = Box::new(predicate) as FilterFn<T>;
        let (sx, rx) = channel();
        self.subscribe_timeout.send((deadline, boxed, sx)).unwrap();
        rx
    }
}

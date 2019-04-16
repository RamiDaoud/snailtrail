// Copyright 2017 ETH Zurich. All rights reserved.
//
// Licensed under the Apache License, Version 2.0 <LICENSE-APACHE or
// http://www.apache.org/licenses/LICENSE-2.0> or the MIT license
// <LICENSE-MIT or http://opensource.org/licenses/MIT>, at your
// option. This file may not be copied, modified, or distributed
// except according to those terms.

//! Betweenness centrality computation traits.
use std::hash::Hash;
use std::fmt::Debug;
use std::ops::{AddAssign, Mul};
use std::cmp::PartialOrd;
use std::time::Duration;

use timely::Data;
use timely::ExchangeData;
use timely::dataflow::{Stream, Scope};
use timely::dataflow::channels::pact::Pipeline;
use timely::dataflow::operators::*;
use timely::dataflow::operators::aggregation::Aggregate;

use crate::hash_code;
use crate::exploration::Capacity;
use crate::exploration::groupexplore::GroupExplore;
use crate::graph::{SrcDst, Partitioning};


/// Trait describing the operation `self * other * multiplicand`.
pub trait ScaleReduce {
    /// Reduce the value of `self` with `other` and scale it with `multiplicand`.
    fn scale_reduce(self, other: Self, multiplicand: usize) -> Self;
}

impl ScaleReduce for u64 {
    fn scale_reduce(self, other: u64, mult: usize) -> Self {
        self * other * mult as u64
    }
}

impl ScaleReduce for f64 {
    fn scale_reduce(self, other: f64, mult: usize) -> Self {
        self * other * mult as f64
    }
}

pub trait ExtendedData: Data + Eq + Hash + Copy + Debug {}
impl<T: Data + Eq + Hash + Copy + Debug> ExtendedData for T {}

/// Methods to push stream timestamps around
pub trait PushTime<S: Scope<Timestamp = Duration>, D: Data> {
    /// Pushes a tuple's timestamp into its data, e.g. for exchanging based on timestamp
    fn push_time(&self) -> Stream<S, (Duration, D)>;
}

impl<S: Scope<Timestamp = Duration>, D: Data> PushTime<S, D> for Stream<S, D> {
    fn push_time(&self) -> Stream<S, (Duration, D)> {
        let mut vector = Vec::new();
        self.unary(Pipeline, "PushTime", move |_, _| {
            move |input, output| {
                input.for_each(|cap, data| {
                    data.swap(&mut vector);
                    let mut session = output.session(&cap);
                    for datum in vector.drain(..) {
                        session.give((cap.time().clone(), datum));
                    }
                });
            }
        })
    }
}

/// Compute the edge betweeness centrality for a generic graph.
pub trait BetweennessCentrality<G, N, D1>
    where G: Scope<Timestamp = Duration>,
          N: ExtendedData + Partitioning,
          D1: SrcDst<N> + Data + Eq + Hash
{
    /// Explores a graph.
    ///
    /// #Examples
    ///
    /// ```
    /// ```

    /// Compute a stream of `(edge, centrality)` for a stream of forward and backward entry points
    ///
    /// The graph is expected to be specified by the implementation.
    fn betweenness_centrality<E, DO>(&self,
                                     forward_edges: &Stream<G, (D1, DO)>,
                                     backward_edges: &Stream<G, (D1, DO)>,
                                     name: &str)
                                     -> Stream<G, (D1, DO)>
        where E: Capacity<D1, DO>,
              DO: ExchangeData + AddAssign + Debug + Copy + Default + Mul + PartialOrd + ScaleReduce;
}

impl<G, N, D1> BetweennessCentrality<G, N, D1> for Stream<G, D1>
    where G: Scope<Timestamp = Duration>,
          N: ExtendedData + Partitioning,
          D1: SrcDst<N> + Data + Eq + Hash + Debug + Send + ExchangeData
{
    fn betweenness_centrality<E, DO>(&self,
                                     forward_edges: &Stream<G, (D1, DO)>,
                                     backward_edges: &Stream<G, (D1, DO)>,
                                     name: &str)
                                     -> Stream<G, (D1, DO)>
        where E: Capacity<D1, DO>,
              DO: ExchangeData + AddAssign + Debug + Copy + Default + Mul + PartialOrd + ScaleReduce
    {
        let forward_edges = forward_edges.push_time().exchange(|x| x.0.as_nanos() as u64).map(|x| x.1);
        let backward_edges = backward_edges.push_time().exchange(|x| x.0.as_nanos() as u64).map(|x| x.1);
        let graph_stream = self.push_time().exchange(|x| x.0.as_nanos() as u64).map(|x| x.1);

        let graph_stream_fwd = graph_stream.concat(&forward_edges.map(|(e, _)| e));
        let graph_stream_bwd = graph_stream.concat(&backward_edges.map(|(e, _)| e));

        let output = graph_stream_fwd.group_explore::<E, _, _>(&forward_edges,
                                                               format!("{} Forward", name)
                                                                   .as_str(),
                                                               |e| e.src(),
                                                               |e| e.dst());

        let output2 = graph_stream_bwd.group_explore::<E, _, _>(&backward_edges,
                                                                format!("{} Backward", name)
                                                                    .as_str(),
                                                                |e| e.dst(),
                                                                |e| e.src());

        // concatenate the two outputs
        let combined = output.concat(&output2);

        // Compute betweeness centrality
        let combined = combined.filter(|&(ref e, _)| e.src().is_some() && e.dst().is_some());
        combined.aggregate::<_,Vec<DO>,_,_,_>(
            |_key, val, agg| agg.push(val),
            |key, mut agg| {
                agg.sort_by(|a, b| a.partial_cmp(b).unwrap());
                match agg.len() {
                    // If this panic triggers, try
                    // 1 => (key, 0)
                    // It means that one exploration produced an edge the other one did not produce,
                    // which means the graph is disconnected and thus should not happen. Could be
                    // I forgot a case. -MH
                    1 => panic!("Wrong number of output tuples, n={}, agg={:?}, key={:?}!", 1, agg, key),
                    // [a, a, b, b] for two edges, so centrality is
                    // a*b + a*b = 2*a*b
                    n => {
                        // Check for even number of edges.
                        // This is only partially correct and won't detect when there's an even number of edges from one side only!
                        // For this to work, we would need to know the direction of the edge.
                        // Idea: Map the output to (edge, (direction, count)) -MH
                        assert_eq!(0, n & 1, "Wrong number of output tuples, n={}, agg={:?}, key={:?}!", n, agg, key);
                        (key, (agg[0].scale_reduce(agg[n/2], n / 2)))
                    },
                }
            },
            |key| hash_code(key))
    }
}

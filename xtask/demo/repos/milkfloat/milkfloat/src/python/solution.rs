//! Solutions as Python sees them.

use crate::model::Stop;
use pyo3::prelude::*;

#[pyclass(name = "Route", frozen, get_all)]
pub struct PyRoute {
    pub vehicle: String,
    pub stops: Vec<String>,
    pub arrivals: Vec<u32>,
}

#[pyclass(name = "Solution", frozen, get_all)]
pub struct PySolution {
    pub routes: Vec<Py<PyRoute>>,
    pub unassigned: Vec<String>,
    pub cost: f64,
}

pub fn stop_name(stop: &Stop) -> String {
    match stop {
        Stop::Depot(_) => "depot".to_owned(),
        Stop::Visit(v) => v.id.clone(),
    }
}

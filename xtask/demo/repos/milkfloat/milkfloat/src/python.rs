//! The Python bindings: `milkfloat._milkfloat`, which `milkfloat` re-exports.

use crate::model::{Location, Problem, TimeWindow, Vehicle, Visit};
use pyo3::exceptions::PyValueError;
use pyo3::prelude::*;
use std::time::Duration;

#[pyclass(name = "TimeWindow", frozen)]
#[derive(Clone)]
pub struct PyTimeWindow(pub TimeWindow);

#[pymethods]
impl PyTimeWindow {
    #[new]
    #[pyo3(signature = (start, end, penalty = None))]
    fn new(start: u32, end: u32, penalty: Option<f64>) -> Self {
        Self(TimeWindow { start, end, penalty })
    }
}

#[pyclass(name = "Vehicle", frozen)]
#[derive(Clone)]
pub struct PyVehicle(pub Vehicle);

#[pymethods]
impl PyVehicle {
    #[new]
    #[pyo3(signature = (id, depot, shift, capacity))]
    fn new(id: String, depot: (f64, f64), shift: PyTimeWindow, capacity: Vec<u32>) -> Self {
        Self(Vehicle { id, depot: Location { lat: depot.0, lon: depot.1 }, shift: shift.0, capacity })
    }
}

#[pyclass(name = "Visit", frozen)]
#[derive(Clone)]
pub struct PyVisit(pub Visit);

#[pymethods]
impl PyVisit {
    #[new]
    #[pyo3(signature = (id, location, *, window = None, service = 0, demand = Vec::new()))]
    fn new(id: String, location: (f64, f64), window: Option<PyTimeWindow>, service: u32, demand: Vec<u32>) -> Self {
        Self(Visit { id, location: Location { lat: location.0, lon: location.1 }, window: window.map(|w| w.0), service, demand })
    }
}

#[pyfunction]
#[pyo3(signature = (vehicles, visits, *, seconds = 10.0, seed = None))]
fn solve(py: Python<'_>, vehicles: Vec<PyVehicle>, visits: Vec<PyVisit>, seconds: f64, seed: Option<u64>) -> PyResult<crate::python::solution::PySolution> {
    let problem = Problem { vehicles: vehicles.into_iter().map(|v| v.0).collect(), visits: visits.into_iter().map(|v| v.0).collect() };
    let time = Duration::from_secs_f64(seconds);
    py.detach(|| crate::solve(&problem, time, seed.unwrap_or(0)))
        .map(Into::into)
        .map_err(|e| PyValueError::new_err(e.to_string()))
}

#[pymodule]
fn _milkfloat(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_class::<PyTimeWindow>()?;
    m.add_class::<PyVehicle>()?;
    m.add_class::<PyVisit>()?;
    m.add_function(wrap_pyfunction!(solve, m)?)?;
    Ok(())
}

mod solution;

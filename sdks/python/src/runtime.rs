use std::sync::Arc;

use boxlite::BoxliteRuntime;
use pyo3::prelude::*;

use crate::box_handle::PyBox;
use crate::info::PyBoxInfo;
use crate::metrics::PyRuntimeMetrics;
use crate::options::{PyBoxOptions, PyOptions};
use crate::util::map_err;

#[pyclass(name = "Boxlite")]
pub(crate) struct PyBoxlite {
    pub(crate) runtime: Arc<BoxliteRuntime>,
}

#[pymethods]
impl PyBoxlite {
    #[new]
    fn new(options: PyOptions) -> PyResult<Self> {
        let runtime = BoxliteRuntime::new(options.into()).map_err(map_err)?;

        Ok(Self {
            runtime: Arc::new(runtime),
        })
    }

    #[staticmethod]
    fn default() -> PyResult<Self> {
        let runtime = BoxliteRuntime::default_runtime();
        Ok(Self {
            runtime: Arc::new(runtime.clone()),
        })
    }

    #[staticmethod]
    fn init_default(options: PyOptions) -> PyResult<()> {
        BoxliteRuntime::init_default_runtime(options.into()).map_err(map_err)
    }

    fn create(&self, options: PyBoxOptions) -> PyResult<PyBox> {
        let (_id, handle) = self.runtime.create(options.into()).map_err(map_err)?;

        Ok(PyBox {
            handle: Arc::new(handle),
        })
    }

    #[pyo3(signature = (_state=None))]
    fn list(&self, _state: Option<String>) -> PyResult<Vec<PyBoxInfo>> {
        let infos = self.runtime.list().map_err(map_err)?;

        Ok(infos.into_iter().map(PyBoxInfo::from).collect())
    }

    fn get_info(&self, box_id: String) -> PyResult<Option<PyBoxInfo>> {
        Ok(self
            .runtime
            .get(&box_id)
            .map_err(map_err)?
            .map(PyBoxInfo::from))
    }

    fn remove(&self, box_id: String) -> PyResult<()> {
        self.runtime.remove(&box_id).map_err(map_err)
    }

    fn metrics(&self) -> PyResult<PyRuntimeMetrics> {
        let metrics = self.runtime.metrics();
        Ok(PyRuntimeMetrics::from(metrics))
    }

    fn close(&self) -> PyResult<()> {
        Ok(())
    }

    fn __enter__(slf: PyRef<'_, Self>) -> PyResult<PyRef<'_, Self>> {
        Ok(slf)
    }

    fn __exit__(
        &self,
        _exc_type: Py<PyAny>,
        _exc_val: Py<PyAny>,
        _exc_tb: Py<PyAny>,
    ) -> PyResult<()> {
        self.close()
    }

    fn __repr__(&self) -> String {
        "Boxlite(open=true)".to_string()
    }
}

use wasm_bindgen::prelude::*;
use rawkit_core::{Graph, Value};
use rawkit_vectors::VectorIndex;

/// The main Rawkit instance for browser use.
#[wasm_bindgen]
pub struct Rawkit {
    graph: Graph,
    vectors: VectorIndex,
}

#[wasm_bindgen]
impl Rawkit {
    /// Create a new Rawkit instance with in-memory storage.
    #[wasm_bindgen(constructor)]
    pub fn new(vector_dimensions: Option<usize>) -> Self {
        Rawkit {
            graph: Graph::in_memory(),
            vectors: VectorIndex::new(vector_dimensions.unwrap_or(384)),
        }
    }

    /// Write a property to a node.
    pub fn put(&self, soul: &str, key: &str, value: JsValue) -> Result<(), JsValue> {
        let val = js_to_value(value)?;
        self.graph.put(soul, key, val);
        Ok(())
    }

    /// Read a property from a node. Returns null if not found.
    pub fn get(&self, soul: &str, key: &str) -> JsValue {
        match self.graph.get(soul, key) {
            Some(val) => value_to_js(&val),
            None => JsValue::NULL,
        }
    }

    /// Write a property with an explicit HAM state timestamp.
    /// Use this when applying incoming sync messages so conflict resolution
    /// uses the sender's clock, not the local browser time.
    pub fn put_with_state(&self, soul: &str, key: &str, value: JsValue, state: f64) -> Result<(), JsValue> {
        let val = js_to_value(value)?;
        self.graph.put_with_state(soul, key, val, state);
        Ok(())
    }

    /// Get all properties of a node as a JS object. Returns null if not found.
    pub fn get_node(&self, soul: &str) -> JsValue {
        match self.graph.get_node(soul) {
            Some(node) => {
                let obj = js_sys::Object::new();
                for (key, val) in &node.props {
                    let _ = js_sys::Reflect::set(&obj, &JsValue::from_str(key), &value_to_js(val));
                }
                obj.into()
            }
            None => JsValue::NULL,
        }
    }

    /// Delete a property.
    pub fn delete(&self, soul: &str, key: &str) {
        self.graph.delete(soul, key);
    }

    /// List nodes by prefix.
    pub fn list(&self, prefix: &str) -> JsValue {
        let souls = self.graph.list(prefix);
        serde_wasm_bindgen::to_value(&souls).unwrap_or(JsValue::NULL)
    }

    /// Add a vector embedding for a node.
    pub fn upsert_vector(&self, soul: &str, embedding: Vec<f32>) -> Result<(), JsValue> {
        self.vectors
            .upsert(soul, embedding)
            .map_err(|e| JsValue::from_str(&e.to_string()))
    }

    /// Search for similar vectors. Returns JSON array of {soul, score}.
    pub fn search_vectors(&self, query: Vec<f32>, top_k: usize) -> Result<JsValue, JsValue> {
        let results = self
            .vectors
            .search(&query, top_k)
            .map_err(|e| JsValue::from_str(&e.to_string()))?;
        serde_wasm_bindgen::to_value(&results).map_err(|e| JsValue::from_str(&e.to_string()))
    }
}

fn js_to_value(js: JsValue) -> Result<Value, JsValue> {
    if js.is_null() || js.is_undefined() {
        Ok(Value::Null)
    } else if let Some(b) = js.as_bool() {
        Ok(Value::Bool(b))
    } else if let Some(n) = js.as_f64() {
        Ok(Value::Number(n))
    } else if let Some(s) = js.as_string() {
        Ok(Value::Text(s))
    } else {
        Err(JsValue::from_str("unsupported value type"))
    }
}

fn value_to_js(val: &Value) -> JsValue {
    match val {
        Value::Null => JsValue::NULL,
        Value::Bool(b) => JsValue::from_bool(*b),
        Value::Number(n) => JsValue::from_f64(*n),
        Value::Text(s) => JsValue::from_str(s),
        Value::Link(link) => JsValue::from_str(&format!("~{}", link.soul)),
    }
}

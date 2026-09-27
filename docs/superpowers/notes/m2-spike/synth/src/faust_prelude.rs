//! The handful of types Faust's Rust backend expects its host to provide.
#![allow(dead_code)]

pub type FaustFloat = f32;
pub type F32 = f32;
pub type F64 = f64;

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct ParamIndex(pub i32);

pub trait Meta {
    fn declare(&mut self, key: &str, value: &str);
}

pub trait UI<T> {
    fn open_tab_box(&mut self, _label: &str) {}
    fn open_horizontal_box(&mut self, _label: &str) {}
    fn open_vertical_box(&mut self, _label: &str) {}
    fn close_box(&mut self) {}
    fn add_button(&mut self, _label: &str, _param: ParamIndex) {}
    fn add_check_button(&mut self, _label: &str, _param: ParamIndex) {}
    fn add_vertical_slider(&mut self, _l: &str, _p: ParamIndex, _i: T, _min: T, _max: T, _s: T) {}
    fn add_horizontal_slider(&mut self, _l: &str, _p: ParamIndex, _i: T, _min: T, _max: T, _s: T) {}
    fn add_num_entry(&mut self, _l: &str, _p: ParamIndex, _i: T, _min: T, _max: T, _s: T) {}
    fn add_horizontal_bargraph(&mut self, _l: &str, _p: ParamIndex, _min: T, _max: T) {}
    fn add_vertical_bargraph(&mut self, _l: &str, _p: ParamIndex, _min: T, _max: T) {}
    fn declare(&mut self, _param: Option<ParamIndex>, _key: &str, _value: &str) {}
}

pub trait FaustDsp {
    type T;
    fn new() -> Self where Self: Sized;
    fn metadata(&self, m: &mut dyn Meta);
    fn get_sample_rate(&self) -> i32;
    fn get_num_inputs(&self) -> i32;
    fn get_num_outputs(&self) -> i32;
    fn class_init(sample_rate: i32) where Self: Sized;
    fn instance_reset_params(&mut self);
    fn instance_clear(&mut self);
    fn instance_constants(&mut self, sample_rate: i32);
    fn instance_init(&mut self, sample_rate: i32);
    fn init(&mut self, sample_rate: i32);
    fn build_user_interface(&self, ui_interface: &mut dyn UI<Self::T>);
    fn build_user_interface_static(ui_interface: &mut dyn UI<Self::T>) where Self: Sized;
    fn get_param(&self, param: ParamIndex) -> Option<Self::T>;
    fn set_param(&mut self, param: ParamIndex, value: Self::T);
    fn compute(&mut self, count: i32, inputs: &[&[Self::T]], outputs: &mut [&mut [Self::T]]);
}

/// Collects `label -> ParamIndex` so a host can set Faust sliders by name.
#[derive(Default)]
pub struct Params(pub std::collections::HashMap<String, ParamIndex>);

impl UI<f32> for Params {
    fn add_button(&mut self, l: &str, p: ParamIndex) {
        self.0.insert(l.to_string(), p);
    }
    fn add_horizontal_slider(&mut self, l: &str, p: ParamIndex, _: f32, _: f32, _: f32, _: f32) {
        self.0.insert(l.to_string(), p);
    }
}

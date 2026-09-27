/* ------------------------------------------------------------
name: "chip"
Code generated with Faust 2.88.0 (https://faust.grame.fr)
Compilation options: -lang rust -fpga-mem-th 4 -ct 1 -cn ChipDsp -es 1 -mcd 16 -mdd 1024 -mdy 33 -single -ftz 0
------------------------------------------------------------ */

#[repr(C)]
pub struct ChipDsp {
	fHslider0: FaustFloat,
	fSampleRate: i32,
	fConst0: F32,
	fConst1: F32,
	fButton0: FaustFloat,
	fVec0: [F32;2],
	iRec1: [i32;2],
	fConst2: F32,
	fConst3: F32,
	fHslider1: FaustFloat,
	iRec2: [i32;2],
	fRec0: [F32;3],
	fHslider2: FaustFloat,
	fHslider3: FaustFloat,
	fConst4: F32,
	fHslider4: FaustFloat,
	fConst5: F32,
	fRec3: [F32;3],
	fHslider5: FaustFloat,
	fHslider6: FaustFloat,
	fHslider7: FaustFloat,
	fRec4: [F32;3],
	fHslider8: FaustFloat,
	fHslider9: FaustFloat,
	fHslider10: FaustFloat,
	fRec5: [F32;3],
	fHslider11: FaustFloat,
	fHslider12: FaustFloat,
	fHslider13: FaustFloat,
	fRec6: [F32;3],
	fHslider14: FaustFloat,
	fHslider15: FaustFloat,
	fHslider16: FaustFloat,
	fRec7: [F32;3],
	fHslider17: FaustFloat,
	fHslider18: FaustFloat,
	fHslider19: FaustFloat,
	fRec8: [F32;3],
	fHslider20: FaustFloat,
	fHslider21: FaustFloat,
	fHslider22: FaustFloat,
	fRec9: [F32;3],
	fHslider23: FaustFloat,
	fHslider24: FaustFloat,
	fHslider25: FaustFloat,
	fRec10: [F32;3],
	fHslider26: FaustFloat,
	fHslider27: FaustFloat,
	fHslider28: FaustFloat,
	fRec11: [F32;3],
	fHslider29: FaustFloat,
	fHslider30: FaustFloat,
	fHslider31: FaustFloat,
	fRec12: [F32;3],
	fHslider32: FaustFloat,
	fHslider33: FaustFloat,
	fHslider34: FaustFloat,
	fRec13: [F32;3],
	fHslider35: FaustFloat,
	fHslider36: FaustFloat,
	fHslider37: FaustFloat,
	fRec14: [F32;3],
	fHslider38: FaustFloat,
}


fn ChipDsp_faustpower2_f(value: F32) -> F32 {
	return value * value;
}
pub const FAUST_INPUTS: usize = 0;
pub const FAUST_OUTPUTS: usize = 1;
pub const FAUST_ACTIVES: usize = 40;
pub const FAUST_PASSIVES: usize = 0;

impl ChipDsp {
		
	pub fn new() -> ChipDsp { 
		ChipDsp {
			fHslider0: 0.0,
			fSampleRate: 0,
			fConst0: 0.0,
			fConst1: 0.0,
			fButton0: 0.0,
			fVec0: [0.0;2],
			iRec1: [0;2],
			fConst2: 0.0,
			fConst3: 0.0,
			fHslider1: 0.0,
			iRec2: [0;2],
			fRec0: [0.0;3],
			fHslider2: 0.0,
			fHslider3: 0.0,
			fConst4: 0.0,
			fHslider4: 0.0,
			fConst5: 0.0,
			fRec3: [0.0;3],
			fHslider5: 0.0,
			fHslider6: 0.0,
			fHslider7: 0.0,
			fRec4: [0.0;3],
			fHslider8: 0.0,
			fHslider9: 0.0,
			fHslider10: 0.0,
			fRec5: [0.0;3],
			fHslider11: 0.0,
			fHslider12: 0.0,
			fHslider13: 0.0,
			fRec6: [0.0;3],
			fHslider14: 0.0,
			fHslider15: 0.0,
			fHslider16: 0.0,
			fRec7: [0.0;3],
			fHslider17: 0.0,
			fHslider18: 0.0,
			fHslider19: 0.0,
			fRec8: [0.0;3],
			fHslider20: 0.0,
			fHslider21: 0.0,
			fHslider22: 0.0,
			fRec9: [0.0;3],
			fHslider23: 0.0,
			fHslider24: 0.0,
			fHslider25: 0.0,
			fRec10: [0.0;3],
			fHslider26: 0.0,
			fHslider27: 0.0,
			fHslider28: 0.0,
			fRec11: [0.0;3],
			fHslider29: 0.0,
			fHslider30: 0.0,
			fHslider31: 0.0,
			fRec12: [0.0;3],
			fHslider32: 0.0,
			fHslider33: 0.0,
			fHslider34: 0.0,
			fRec13: [0.0;3],
			fHslider35: 0.0,
			fHslider36: 0.0,
			fHslider37: 0.0,
			fRec14: [0.0;3],
			fHslider38: 0.0,
		}
	}
	pub fn metadata(&self, m: &mut dyn Meta) { 
		m.declare("compile_options", r"-lang rust -fpga-mem-th 4 -ct 1 -cn ChipDsp -es 1 -mcd 16 -mdd 1024 -mdy 33 -single -ftz 0");
		m.declare("envelopes.lib/ar:author", r"Yann Orlarey, Stéphane Letz");
		m.declare("envelopes.lib/author", r"GRAME");
		m.declare("envelopes.lib/copyright", r"GRAME");
		m.declare("envelopes.lib/license", r"LicenseRef-LGPL-2.1-or-later-with-Faust-exception");
		m.declare("envelopes.lib/name", r"Faust Envelope Library");
		m.declare("envelopes.lib/version", r"1.3.0");
		m.declare("filename", r"chip.dsp");
		m.declare("filters.lib/fir:author", r"Julius O. Smith III");
		m.declare("filters.lib/fir:copyright", r"Copyright (C) 2003-2019 by Julius O. Smith III <jos@ccrma.stanford.edu>");
		m.declare("filters.lib/fir:license", r"LicenseRef-STK-4.3");
		m.declare("filters.lib/iir:author", r"Julius O. Smith III");
		m.declare("filters.lib/iir:copyright", r"Copyright (C) 2003-2019 by Julius O. Smith III <jos@ccrma.stanford.edu>");
		m.declare("filters.lib/iir:license", r"LicenseRef-STK-4.3");
		m.declare("filters.lib/lowpass0_highpass1:author", r"Julius O. Smith III");
		m.declare("filters.lib/lowpass0_highpass1:copyright", r"Copyright (C) 2003-2019 by Julius O. Smith III <jos@ccrma.stanford.edu>");
		m.declare("filters.lib/lowpass0_highpass1:license", r"LicenseRef-STK-4.3");
		m.declare("filters.lib/lowpass:author", r"Julius O. Smith III");
		m.declare("filters.lib/lowpass:copyright", r"Copyright (C) 2003-2019 by Julius O. Smith III <jos@ccrma.stanford.edu>");
		m.declare("filters.lib/lowpass:license", r"LicenseRef-STK-4.3");
		m.declare("filters.lib/name", r"Faust Filters Library");
		m.declare("filters.lib/tf2:author", r"Julius O. Smith III");
		m.declare("filters.lib/tf2:copyright", r"Copyright (C) 2003-2019 by Julius O. Smith III <jos@ccrma.stanford.edu>");
		m.declare("filters.lib/tf2:license", r"LicenseRef-STK-4.3");
		m.declare("filters.lib/tf2s:author", r"Julius O. Smith III");
		m.declare("filters.lib/tf2s:copyright", r"Copyright (C) 2003-2019 by Julius O. Smith III <jos@ccrma.stanford.edu>");
		m.declare("filters.lib/tf2s:license", r"LicenseRef-STK-4.3");
		m.declare("filters.lib/version", r"1.9.0");
		m.declare("maths.lib/author", r"GRAME");
		m.declare("maths.lib/copyright", r"GRAME");
		m.declare("maths.lib/license", r"LicenseRef-LGPL-2.1-or-later-with-Faust-exception");
		m.declare("maths.lib/name", r"Faust Math Library");
		m.declare("maths.lib/version", r"2.9.0");
		m.declare("name", r"chip");
		m.declare("noises.lib/name", r"Faust Noise Generator Library");
		m.declare("noises.lib/version", r"1.6.0");
		m.declare("physmodels.lib/name", r"Faust Physical Models Library");
		m.declare("physmodels.lib/version", r"1.2.0");
		m.declare("platform.lib/name", r"Generic Platform Library");
		m.declare("platform.lib/version", r"1.3.0");
	}

	pub fn get_sample_rate(&self) -> i32 { self.fSampleRate as i32}
	
	pub fn class_init(sample_rate: i32) {
		// Obtaining locks on 0 static var(s)
	}
	pub fn instance_reset_params(&mut self) {
		self.fHslider0 = (9e+03) as FaustFloat;
		self.fButton0 = (0.0) as FaustFloat;
		self.fHslider1 = (0.0008) as FaustFloat;
		self.fHslider2 = (0.3) as FaustFloat;
		self.fHslider3 = (0.05) as FaustFloat;
		self.fHslider4 = (5e+03) as FaustFloat;
		self.fHslider5 = (0.1) as FaustFloat;
		self.fHslider6 = (0.05) as FaustFloat;
		self.fHslider7 = (5e+03) as FaustFloat;
		self.fHslider8 = (0.1) as FaustFloat;
		self.fHslider9 = (0.05) as FaustFloat;
		self.fHslider10 = (5e+03) as FaustFloat;
		self.fHslider11 = (0.1) as FaustFloat;
		self.fHslider12 = (0.05) as FaustFloat;
		self.fHslider13 = (5e+03) as FaustFloat;
		self.fHslider14 = (0.1) as FaustFloat;
		self.fHslider15 = (0.05) as FaustFloat;
		self.fHslider16 = (5e+03) as FaustFloat;
		self.fHslider17 = (0.1) as FaustFloat;
		self.fHslider18 = (0.05) as FaustFloat;
		self.fHslider19 = (5e+03) as FaustFloat;
		self.fHslider20 = (0.1) as FaustFloat;
		self.fHslider21 = (0.05) as FaustFloat;
		self.fHslider22 = (5e+03) as FaustFloat;
		self.fHslider23 = (0.1) as FaustFloat;
		self.fHslider24 = (0.05) as FaustFloat;
		self.fHslider25 = (5e+03) as FaustFloat;
		self.fHslider26 = (0.1) as FaustFloat;
		self.fHslider27 = (0.05) as FaustFloat;
		self.fHslider28 = (5e+03) as FaustFloat;
		self.fHslider29 = (0.1) as FaustFloat;
		self.fHslider30 = (0.05) as FaustFloat;
		self.fHslider31 = (5e+03) as FaustFloat;
		self.fHslider32 = (0.1) as FaustFloat;
		self.fHslider33 = (0.05) as FaustFloat;
		self.fHslider34 = (5e+03) as FaustFloat;
		self.fHslider35 = (0.1) as FaustFloat;
		self.fHslider36 = (0.05) as FaustFloat;
		self.fHslider37 = (5e+03) as FaustFloat;
		self.fHslider38 = (0.1) as FaustFloat;
	}
	pub fn instance_clear(&mut self) {
		for l0 in 0..2 {
			self.fVec0[l0 as usize] = 0.0;
		}
		for l1 in 0..2 {
			self.iRec1[l1 as usize] = 0;
		}
		for l2 in 0..2 {
			self.iRec2[l2 as usize] = 0;
		}
		for l3 in 0..3 {
			self.fRec0[l3 as usize] = 0.0;
		}
		for l4 in 0..3 {
			self.fRec3[l4 as usize] = 0.0;
		}
		for l5 in 0..3 {
			self.fRec4[l5 as usize] = 0.0;
		}
		for l6 in 0..3 {
			self.fRec5[l6 as usize] = 0.0;
		}
		for l7 in 0..3 {
			self.fRec6[l7 as usize] = 0.0;
		}
		for l8 in 0..3 {
			self.fRec7[l8 as usize] = 0.0;
		}
		for l9 in 0..3 {
			self.fRec8[l9 as usize] = 0.0;
		}
		for l10 in 0..3 {
			self.fRec9[l10 as usize] = 0.0;
		}
		for l11 in 0..3 {
			self.fRec10[l11 as usize] = 0.0;
		}
		for l12 in 0..3 {
			self.fRec11[l12 as usize] = 0.0;
		}
		for l13 in 0..3 {
			self.fRec12[l13 as usize] = 0.0;
		}
		for l14 in 0..3 {
			self.fRec13[l14 as usize] = 0.0;
		}
		for l15 in 0..3 {
			self.fRec14[l15 as usize] = 0.0;
		}
	}
	pub fn instance_constants(&mut self, sample_rate: i32) {
		// Obtaining locks on 0 static var(s)
		self.fSampleRate = sample_rate;
		self.fConst0 = F32::min(1.92e+05, F32::max(1.0, (self.fSampleRate) as F32));
		self.fConst1 = 3.1415927 / self.fConst0;
		self.fConst2 = F32::max(1.0, 5e-05 * self.fConst0);
		self.fConst3 = 1.0 / self.fConst2;
		self.fConst4 = 1.0 / self.fConst0;
		self.fConst5 = 6.2831855 / self.fConst0;
	}
	pub fn instance_init(&mut self, sample_rate: i32) {
		self.instance_constants(sample_rate);
		self.instance_reset_params();
		self.instance_clear();
	}
	pub fn init(&mut self, sample_rate: i32) {
		ChipDsp::class_init(sample_rate);
		self.instance_init(sample_rate);
	}
	
	pub fn build_user_interface(&self, ui_interface: &mut dyn UI<FaustFloat>) {
		Self::build_user_interface_static(ui_interface);
	}
	
	pub fn build_user_interface_static(ui_interface: &mut dyn UI<FaustFloat>) {
		ui_interface.open_vertical_box("chip");
		ui_interface.add_horizontal_slider("bright", ParamIndex(0), 9e+03, 5e+02, 2e+04, 1.0);
		ui_interface.add_horizontal_slider("contact", ParamIndex(1), 0.3, 0.0, 1.0, 0.001);
		ui_interface.add_horizontal_slider("f0", ParamIndex(2), 5e+03, 1e+02, 2e+04, 1.0);
		ui_interface.add_horizontal_slider("f1", ParamIndex(3), 5e+03, 1e+02, 2e+04, 1.0);
		ui_interface.add_horizontal_slider("f10", ParamIndex(4), 5e+03, 1e+02, 2e+04, 1.0);
		ui_interface.add_horizontal_slider("f11", ParamIndex(5), 5e+03, 1e+02, 2e+04, 1.0);
		ui_interface.add_horizontal_slider("f2", ParamIndex(6), 5e+03, 1e+02, 2e+04, 1.0);
		ui_interface.add_horizontal_slider("f3", ParamIndex(7), 5e+03, 1e+02, 2e+04, 1.0);
		ui_interface.add_horizontal_slider("f4", ParamIndex(8), 5e+03, 1e+02, 2e+04, 1.0);
		ui_interface.add_horizontal_slider("f5", ParamIndex(9), 5e+03, 1e+02, 2e+04, 1.0);
		ui_interface.add_horizontal_slider("f6", ParamIndex(10), 5e+03, 1e+02, 2e+04, 1.0);
		ui_interface.add_horizontal_slider("f7", ParamIndex(11), 5e+03, 1e+02, 2e+04, 1.0);
		ui_interface.add_horizontal_slider("f8", ParamIndex(12), 5e+03, 1e+02, 2e+04, 1.0);
		ui_interface.add_horizontal_slider("f9", ParamIndex(13), 5e+03, 1e+02, 2e+04, 1.0);
		ui_interface.add_horizontal_slider("g0", ParamIndex(14), 0.1, 0.0, 1.0, 0.0001);
		ui_interface.add_horizontal_slider("g1", ParamIndex(15), 0.1, 0.0, 1.0, 0.0001);
		ui_interface.add_horizontal_slider("g10", ParamIndex(16), 0.1, 0.0, 1.0, 0.0001);
		ui_interface.add_horizontal_slider("g11", ParamIndex(17), 0.1, 0.0, 1.0, 0.0001);
		ui_interface.add_horizontal_slider("g2", ParamIndex(18), 0.1, 0.0, 1.0, 0.0001);
		ui_interface.add_horizontal_slider("g3", ParamIndex(19), 0.1, 0.0, 1.0, 0.0001);
		ui_interface.add_horizontal_slider("g4", ParamIndex(20), 0.1, 0.0, 1.0, 0.0001);
		ui_interface.add_horizontal_slider("g5", ParamIndex(21), 0.1, 0.0, 1.0, 0.0001);
		ui_interface.add_horizontal_slider("g6", ParamIndex(22), 0.1, 0.0, 1.0, 0.0001);
		ui_interface.add_horizontal_slider("g7", ParamIndex(23), 0.1, 0.0, 1.0, 0.0001);
		ui_interface.add_horizontal_slider("g8", ParamIndex(24), 0.1, 0.0, 1.0, 0.0001);
		ui_interface.add_horizontal_slider("g9", ParamIndex(25), 0.1, 0.0, 1.0, 0.0001);
		ui_interface.add_button("gate", ParamIndex(26));
		ui_interface.add_horizontal_slider("t0", ParamIndex(27), 0.05, 0.001, 2.0, 0.0001);
		ui_interface.add_horizontal_slider("t1", ParamIndex(28), 0.05, 0.001, 2.0, 0.0001);
		ui_interface.add_horizontal_slider("t10", ParamIndex(29), 0.05, 0.001, 2.0, 0.0001);
		ui_interface.add_horizontal_slider("t11", ParamIndex(30), 0.05, 0.001, 2.0, 0.0001);
		ui_interface.add_horizontal_slider("t2", ParamIndex(31), 0.05, 0.001, 2.0, 0.0001);
		ui_interface.add_horizontal_slider("t3", ParamIndex(32), 0.05, 0.001, 2.0, 0.0001);
		ui_interface.add_horizontal_slider("t4", ParamIndex(33), 0.05, 0.001, 2.0, 0.0001);
		ui_interface.add_horizontal_slider("t5", ParamIndex(34), 0.05, 0.001, 2.0, 0.0001);
		ui_interface.add_horizontal_slider("t6", ParamIndex(35), 0.05, 0.001, 2.0, 0.0001);
		ui_interface.add_horizontal_slider("t7", ParamIndex(36), 0.05, 0.001, 2.0, 0.0001);
		ui_interface.add_horizontal_slider("t8", ParamIndex(37), 0.05, 0.001, 2.0, 0.0001);
		ui_interface.add_horizontal_slider("t9", ParamIndex(38), 0.05, 0.001, 2.0, 0.0001);
		ui_interface.add_horizontal_slider("width", ParamIndex(39), 0.0008, 0.0001, 0.01, 1e-05);
		ui_interface.close_box();
	}
	
	pub fn get_param(&self, param: ParamIndex) -> Option<FaustFloat> {
		match param.0 {
			26 => Some(self.fButton0),
			0 => Some(self.fHslider0),
			39 => Some(self.fHslider1),
			13 => Some(self.fHslider10),
			25 => Some(self.fHslider11),
			37 => Some(self.fHslider12),
			12 => Some(self.fHslider13),
			24 => Some(self.fHslider14),
			36 => Some(self.fHslider15),
			11 => Some(self.fHslider16),
			23 => Some(self.fHslider17),
			35 => Some(self.fHslider18),
			10 => Some(self.fHslider19),
			1 => Some(self.fHslider2),
			22 => Some(self.fHslider20),
			34 => Some(self.fHslider21),
			9 => Some(self.fHslider22),
			21 => Some(self.fHslider23),
			33 => Some(self.fHslider24),
			8 => Some(self.fHslider25),
			20 => Some(self.fHslider26),
			32 => Some(self.fHslider27),
			7 => Some(self.fHslider28),
			19 => Some(self.fHslider29),
			30 => Some(self.fHslider3),
			31 => Some(self.fHslider30),
			6 => Some(self.fHslider31),
			18 => Some(self.fHslider32),
			28 => Some(self.fHslider33),
			3 => Some(self.fHslider34),
			15 => Some(self.fHslider35),
			27 => Some(self.fHslider36),
			2 => Some(self.fHslider37),
			14 => Some(self.fHslider38),
			5 => Some(self.fHslider4),
			17 => Some(self.fHslider5),
			29 => Some(self.fHslider6),
			4 => Some(self.fHslider7),
			16 => Some(self.fHslider8),
			38 => Some(self.fHslider9),
			_ => None,
		}
	}
	
	pub fn set_param(&mut self, param: ParamIndex, value: FaustFloat) {
		match param.0 {
			26 => { self.fButton0 = value }
			0 => { self.fHslider0 = value }
			39 => { self.fHslider1 = value }
			13 => { self.fHslider10 = value }
			25 => { self.fHslider11 = value }
			37 => { self.fHslider12 = value }
			12 => { self.fHslider13 = value }
			24 => { self.fHslider14 = value }
			36 => { self.fHslider15 = value }
			11 => { self.fHslider16 = value }
			23 => { self.fHslider17 = value }
			35 => { self.fHslider18 = value }
			10 => { self.fHslider19 = value }
			1 => { self.fHslider2 = value }
			22 => { self.fHslider20 = value }
			34 => { self.fHslider21 = value }
			9 => { self.fHslider22 = value }
			21 => { self.fHslider23 = value }
			33 => { self.fHslider24 = value }
			8 => { self.fHslider25 = value }
			20 => { self.fHslider26 = value }
			32 => { self.fHslider27 = value }
			7 => { self.fHslider28 = value }
			19 => { self.fHslider29 = value }
			30 => { self.fHslider3 = value }
			31 => { self.fHslider30 = value }
			6 => { self.fHslider31 = value }
			18 => { self.fHslider32 = value }
			28 => { self.fHslider33 = value }
			3 => { self.fHslider34 = value }
			15 => { self.fHslider35 = value }
			27 => { self.fHslider36 = value }
			2 => { self.fHslider37 = value }
			14 => { self.fHslider38 = value }
			5 => { self.fHslider4 = value }
			17 => { self.fHslider5 = value }
			29 => { self.fHslider6 = value }
			4 => { self.fHslider7 = value }
			16 => { self.fHslider8 = value }
			38 => { self.fHslider9 = value }
			_ => {}
		}
	}
	
	pub fn compute(
		&mut self,
		count: usize,
		inputs: &[impl AsRef<[FaustFloat]>],
		outputs: &mut[impl AsMut<[FaustFloat]>],
	) {
		
		// Obtaining locks on 0 static var(s)
		let [outputs0, .. ] = outputs.as_mut() else { panic!("wrong number of output buffers"); };
		let outputs0 = outputs0.as_mut()[..count].iter_mut();
		let mut fSlow0: F32 = F32::tan(self.fConst1 * (self.fHslider0) as F32);
		let mut fSlow1: F32 = 2.0 * (1.0 - 1.0 / ChipDsp_faustpower2_f(fSlow0));
		let mut fSlow2: F32 = 1.0 / fSlow0;
		let mut fSlow3: F32 = (fSlow2 + -1.4142135) / fSlow0 + 1.0;
		let mut fSlow4: F32 = (fSlow2 + 1.4142135) / fSlow0 + 1.0;
		let mut fSlow5: F32 = 1.0 / fSlow4;
		let mut fSlow6: F32 = (self.fButton0) as F32;
		let mut fSlow7: F32 = 1.0 / F32::max(1.0, self.fConst0 * (self.fHslider1) as F32);
		let mut fSlow8: F32 = (self.fHslider2) as F32 / fSlow4;
		let mut fSlow9: F32 = F32::powf(0.001, self.fConst4 / (self.fHslider3) as F32);
		let mut fSlow10: F32 = ChipDsp_faustpower2_f(fSlow9);
		let mut fSlow11: F32 = 2.0 * fSlow9 * F32::cos(self.fConst5 * (self.fHslider4) as F32);
		let mut fSlow12: F32 = (self.fHslider5) as F32;
		let mut fSlow13: F32 = F32::powf(0.001, self.fConst4 / (self.fHslider6) as F32);
		let mut fSlow14: F32 = ChipDsp_faustpower2_f(fSlow13);
		let mut fSlow15: F32 = 2.0 * fSlow13 * F32::cos(self.fConst5 * (self.fHslider7) as F32);
		let mut fSlow16: F32 = (self.fHslider8) as F32;
		let mut fSlow17: F32 = F32::powf(0.001, self.fConst4 / (self.fHslider9) as F32);
		let mut fSlow18: F32 = ChipDsp_faustpower2_f(fSlow17);
		let mut fSlow19: F32 = 2.0 * fSlow17 * F32::cos(self.fConst5 * (self.fHslider10) as F32);
		let mut fSlow20: F32 = (self.fHslider11) as F32;
		let mut fSlow21: F32 = F32::powf(0.001, self.fConst4 / (self.fHslider12) as F32);
		let mut fSlow22: F32 = ChipDsp_faustpower2_f(fSlow21);
		let mut fSlow23: F32 = 2.0 * fSlow21 * F32::cos(self.fConst5 * (self.fHslider13) as F32);
		let mut fSlow24: F32 = (self.fHslider14) as F32;
		let mut fSlow25: F32 = F32::powf(0.001, self.fConst4 / (self.fHslider15) as F32);
		let mut fSlow26: F32 = ChipDsp_faustpower2_f(fSlow25);
		let mut fSlow27: F32 = 2.0 * fSlow25 * F32::cos(self.fConst5 * (self.fHslider16) as F32);
		let mut fSlow28: F32 = (self.fHslider17) as F32;
		let mut fSlow29: F32 = F32::powf(0.001, self.fConst4 / (self.fHslider18) as F32);
		let mut fSlow30: F32 = ChipDsp_faustpower2_f(fSlow29);
		let mut fSlow31: F32 = 2.0 * fSlow29 * F32::cos(self.fConst5 * (self.fHslider19) as F32);
		let mut fSlow32: F32 = (self.fHslider20) as F32;
		let mut fSlow33: F32 = F32::powf(0.001, self.fConst4 / (self.fHslider21) as F32);
		let mut fSlow34: F32 = ChipDsp_faustpower2_f(fSlow33);
		let mut fSlow35: F32 = 2.0 * fSlow33 * F32::cos(self.fConst5 * (self.fHslider22) as F32);
		let mut fSlow36: F32 = (self.fHslider23) as F32;
		let mut fSlow37: F32 = F32::powf(0.001, self.fConst4 / (self.fHslider24) as F32);
		let mut fSlow38: F32 = ChipDsp_faustpower2_f(fSlow37);
		let mut fSlow39: F32 = 2.0 * fSlow37 * F32::cos(self.fConst5 * (self.fHslider25) as F32);
		let mut fSlow40: F32 = (self.fHslider26) as F32;
		let mut fSlow41: F32 = F32::powf(0.001, self.fConst4 / (self.fHslider27) as F32);
		let mut fSlow42: F32 = ChipDsp_faustpower2_f(fSlow41);
		let mut fSlow43: F32 = 2.0 * fSlow41 * F32::cos(self.fConst5 * (self.fHslider28) as F32);
		let mut fSlow44: F32 = (self.fHslider29) as F32;
		let mut fSlow45: F32 = F32::powf(0.001, self.fConst4 / (self.fHslider30) as F32);
		let mut fSlow46: F32 = ChipDsp_faustpower2_f(fSlow45);
		let mut fSlow47: F32 = 2.0 * fSlow45 * F32::cos(self.fConst5 * (self.fHslider31) as F32);
		let mut fSlow48: F32 = (self.fHslider32) as F32;
		let mut fSlow49: F32 = F32::powf(0.001, self.fConst4 / (self.fHslider33) as F32);
		let mut fSlow50: F32 = ChipDsp_faustpower2_f(fSlow49);
		let mut fSlow51: F32 = 2.0 * fSlow49 * F32::cos(self.fConst5 * (self.fHslider34) as F32);
		let mut fSlow52: F32 = (self.fHslider35) as F32;
		let mut fSlow53: F32 = F32::powf(0.001, self.fConst4 / (self.fHslider36) as F32);
		let mut fSlow54: F32 = ChipDsp_faustpower2_f(fSlow53);
		let mut fSlow55: F32 = 2.0 * fSlow53 * F32::cos(self.fConst5 * (self.fHslider37) as F32);
		let mut fSlow56: F32 = (self.fHslider38) as F32;
		let zipped_iterators = outputs0;
		for output0 in zipped_iterators {
			self.fVec0[0] = fSlow6;
			self.iRec1[0] = ((fSlow6 > self.fVec0[1]) as i32) + i32::wrapping_mul(i32::wrapping_add(self.iRec1[1], (self.iRec1[1] > 0) as i32), (fSlow6 <= self.fVec0[1]) as i32);
			let mut fTemp0: F32 = (self.iRec1[0]) as F32;
			self.iRec2[0] = i32::wrapping_add(12345, i32::wrapping_mul(1103515245, self.iRec2[1]));
			self.fRec0[0] = 4.656613e-10 * (self.iRec2[0]) as F32 * F32::max(0.0, F32::min(self.fConst3 * fTemp0, 1.0 + fSlow7 * (self.fConst2 - fTemp0))) - fSlow5 * (fSlow3 * self.fRec0[2] + fSlow1 * self.fRec0[1]);
			let mut fTemp1: F32 = self.fRec0[2] + self.fRec0[0] + 2.0 * self.fRec0[1];
			let mut fTemp2: F32 = fSlow5 * fTemp1;
			self.fRec3[0] = fTemp2 + fSlow11 * self.fRec3[1] - fSlow10 * self.fRec3[2];
			self.fRec4[0] = fTemp2 + fSlow15 * self.fRec4[1] - fSlow14 * self.fRec4[2];
			self.fRec5[0] = fTemp2 + fSlow19 * self.fRec5[1] - fSlow18 * self.fRec5[2];
			self.fRec6[0] = fTemp2 + fSlow23 * self.fRec6[1] - fSlow22 * self.fRec6[2];
			self.fRec7[0] = fTemp2 + fSlow27 * self.fRec7[1] - fSlow26 * self.fRec7[2];
			self.fRec8[0] = fTemp2 + fSlow31 * self.fRec8[1] - fSlow30 * self.fRec8[2];
			self.fRec9[0] = fTemp2 + fSlow35 * self.fRec9[1] - fSlow34 * self.fRec9[2];
			self.fRec10[0] = fTemp2 + fSlow39 * self.fRec10[1] - fSlow38 * self.fRec10[2];
			self.fRec11[0] = fTemp2 + fSlow43 * self.fRec11[1] - fSlow42 * self.fRec11[2];
			self.fRec12[0] = fTemp2 + fSlow47 * self.fRec12[1] - fSlow46 * self.fRec12[2];
			self.fRec13[0] = fTemp2 + fSlow51 * self.fRec13[1] - fSlow50 * self.fRec13[2];
			self.fRec14[0] = fTemp2 + fSlow55 * self.fRec14[1] - fSlow54 * self.fRec14[2];
			*output0 = (fSlow56 * (self.fRec14[0] - self.fRec14[2]) + fSlow52 * (self.fRec13[0] - self.fRec13[2]) + fSlow48 * (self.fRec12[0] - self.fRec12[2]) + fSlow44 * (self.fRec11[0] - self.fRec11[2]) + fSlow40 * (self.fRec10[0] - self.fRec10[2]) + fSlow36 * (self.fRec9[0] - self.fRec9[2]) + fSlow32 * (self.fRec8[0] - self.fRec8[2]) + fSlow28 * (self.fRec7[0] - self.fRec7[2]) + fSlow24 * (self.fRec6[0] - self.fRec6[2]) + fSlow20 * (self.fRec5[0] - self.fRec5[2]) + fSlow16 * (self.fRec4[0] - self.fRec4[2]) + fSlow12 * (self.fRec3[0] - self.fRec3[2]) + fSlow8 * fTemp1) as FaustFloat;
			self.fVec0[1] = self.fVec0[0];
			self.iRec1[1] = self.iRec1[0];
			self.iRec2[1] = self.iRec2[0];
			self.fRec0[2] = self.fRec0[1];
			self.fRec0[1] = self.fRec0[0];
			self.fRec3[2] = self.fRec3[1];
			self.fRec3[1] = self.fRec3[0];
			self.fRec4[2] = self.fRec4[1];
			self.fRec4[1] = self.fRec4[0];
			self.fRec5[2] = self.fRec5[1];
			self.fRec5[1] = self.fRec5[0];
			self.fRec6[2] = self.fRec6[1];
			self.fRec6[1] = self.fRec6[0];
			self.fRec7[2] = self.fRec7[1];
			self.fRec7[1] = self.fRec7[0];
			self.fRec8[2] = self.fRec8[1];
			self.fRec8[1] = self.fRec8[0];
			self.fRec9[2] = self.fRec9[1];
			self.fRec9[1] = self.fRec9[0];
			self.fRec10[2] = self.fRec10[1];
			self.fRec10[1] = self.fRec10[0];
			self.fRec11[2] = self.fRec11[1];
			self.fRec11[1] = self.fRec11[0];
			self.fRec12[2] = self.fRec12[1];
			self.fRec12[1] = self.fRec12[0];
			self.fRec13[2] = self.fRec13[1];
			self.fRec13[1] = self.fRec13[0];
			self.fRec14[2] = self.fRec14[1];
			self.fRec14[1] = self.fRec14[0];
		}
		
	}

}

#[cfg(not(target_arch = "wasm32"))] // Compile ffi bindings only on non-wasm targets
mod ffi {
	use core::ffi::c_float;
	// Conditionally compile the link attribute only on non-Windows platforms
	#[cfg_attr(not(target_os = "windows"), link(name = "m"))]
	unsafe extern "C" {
		pub fn remainderf(from: c_float, to: c_float) -> c_float;
		pub fn rintf(val: c_float) -> c_float;
	}
}
fn remainderf(from: f32, to: f32) -> f32 {
	#[cfg(not(target_arch = "wasm32"))] // non-wasm targets use ffi bindings
	unsafe { ffi::remainderf(from, to) }
	#[cfg(target_arch = "wasm32")] // wasm relies on libm
	libm::remainderf(from, to)
}
fn rintf(val: f32) -> f32 {
	#[cfg(not(target_arch = "wasm32"))] // non-wasm targets use ffi bindings
	unsafe { ffi::rintf(val) }
	#[cfg(target_arch = "wasm32")] // wasm relies on libm
	libm::rintf(val)
}

impl FaustDsp for ChipDsp {
	type T = FaustFloat;
	fn new() -> Self where Self: Sized {
		Self::new()
	}
	fn metadata(&self, m: &mut dyn Meta) {
		self.metadata(m)
	}
	fn get_sample_rate(&self) -> i32 {
		self.get_sample_rate()
	}
	fn get_num_inputs(&self) -> i32 {
		FAUST_INPUTS as i32
	}
	fn get_num_outputs(&self) -> i32 {
		FAUST_OUTPUTS as i32
	}
	fn class_init(sample_rate: i32) where Self: Sized {
		Self::class_init(sample_rate);
	}
	fn instance_reset_params(&mut self) {
		self.instance_reset_params()
	}
	fn instance_clear(&mut self) {
		self.instance_clear()
	}
	fn instance_constants(&mut self, sample_rate: i32) {
		self.instance_constants(sample_rate)
	}
	fn instance_init(&mut self, sample_rate: i32) {
		self.instance_init(sample_rate)
	}
	fn init(&mut self, sample_rate: i32) {
		self.init(sample_rate)
	}
	fn build_user_interface(&self, ui_interface: &mut dyn UI<Self::T>) {
		self.build_user_interface(ui_interface)
	}
	fn build_user_interface_static(ui_interface: &mut dyn UI<Self::T>) where Self: Sized {
		Self::build_user_interface_static(ui_interface);
	}
	fn get_param(&self, param: ParamIndex) -> Option<Self::T> {
		self.get_param(param)
	}
	fn set_param(&mut self, param: ParamIndex, value: Self::T) {
		self.set_param(param, value)
	}
	fn compute(&mut self, count: i32, inputs: &[&[Self::T]], outputs: &mut [&mut [Self::T]]) {
		self.compute(count as usize, inputs, outputs)
	}
}

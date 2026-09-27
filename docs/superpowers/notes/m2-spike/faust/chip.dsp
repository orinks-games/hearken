// One chip-on-chip strike: a contact burst exciting a cluster of stiff
// plate modes. Mode frequencies, decays and gains are set per hit by the host.
import("stdfaust.lib");
N = 12;
gate = button("gate");
f(i) = hslider("f%i", 5000, 100, 20000, 1);
t(i) = hslider("t%i", 0.05, 0.001, 2, 0.0001);
g(i) = hslider("g%i", 0.1, 0, 1, 0.0001);
bright = hslider("bright", 9000, 500, 20000, 1);
contact = hslider("contact", 0.3, 0, 1, 0.001);
width = hslider("width", 0.0008, 0.0001, 0.01, 0.00001);
exc = no.noise * en.ar(0.00005, width, gate) : fi.lowpass(2, bright);
process = exc <: (par(i, N, pm.modeFilter(f(i), t(i), g(i))) :> _), _*contact :> _;

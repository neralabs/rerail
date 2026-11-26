//! The GPU lane's prices (accel's `price` bench, the lane's part): the spike-sized workloads on the device, and a
//! submission's fixed cost.

use rerail_accel::exec::process_cpu;
use rerail_accel::kernel::{Form, Kernel};
use rerail_accel::workloads::*;
use std::time::Instant;

#[test]
#[ignore = "prices: release build, run alone with --ignored --nocapture"]
fn gpu() {
    use rerail_accel::exec::Backend;
    header();
    let g = super::gpu_backend().expect("this machine has a Metal device");
    for (name, k) in workloads().into_iter().filter(|(_, k)| g.can(k)) {
        let want = rerail_accel::cpu::run(&k);
        let (out, _) = g.run(&[&k]).unwrap();
        assert!(out[0] == want, "{name}");
        let secs = best(7, || g.run(&[&k]).unwrap().1.device);
        row(name, "gpu (device)", &k, secs, "device");
    }
    // A submission's fixed cost: one tiny launch, wall and host CPU (the device's own time is a few µs).
    let k = tiny(Form::Histogram);
    let (mut cpu, mut dev) = (f64::INFINITY, f64::INFINITY);
    let w = best(50, || {
        let (c0, t) = (process_cpu(), Instant::now());
        let (_, s) = g.run(&[&k]).unwrap();
        let w = t.elapsed().as_secs_f64();
        cpu = cpu.min(process_cpu() - c0);
        dev = dev.min(s.device);
        w
    });
    // The forms one submission holds (`run`): a join runs in its space's own command buffers (`submit`'s `space::run`).
    let ks: Vec<Kernel> = Form::ALL.iter().filter(|f| !matches!(f, Form::Probe | Form::Tile)).cycle().take(20).map(|&f| tiny(f)).collect();
    let refs: Vec<&Kernel> = ks.iter().collect();
    let w20 = best(50, || {
        let t = Instant::now();
        std::hint::black_box(g.run(&refs).unwrap());
        t.elapsed().as_secs_f64()
    });
    println!(
        "fixed: a submission of one tiny launch: wall {:.1} us, host CPU {:.1} us, device {:.1} us; of 20: wall {:.1} us ({:.2} us a launch more)",
        w * 1e6,
        cpu * 1e6,
        dev * 1e6,
        w20 * 1e6,
        (w20 - w) * 1e6 / 19.0
    );
    println!("load after: {}", load());
}

//! 音频引擎：多进程源 + 物理麦 + SFX，fan-out 到多个独立输出设备。

use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use crate::audio::buffer::SpscRing;
use crate::audio::capture::{open_mic_capture, open_process_capture, run_capture_loop};
use crate::audio::com;
use crate::audio::format::{AudioFormat, Converter, SampleKind};
use crate::audio::render::{open_render_device, run_render_loop};
use crate::error::Error;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum EngineStatus { Idle, Running, ProcessExited, DeviceGone, Error(String) }

struct TargetState {
    format: AudioFormat,
    process_rings: Vec<Arc<SpscRing>>,
    mic_ring: Arc<SpscRing>,
    sfx_ring: Arc<SpscRing>,
    render: JoinHandle<()>,
}

struct OutputState {
    stop: Arc<AtomicBool>,
    process_rings: Vec<Arc<SpscRing>>,
    mic_ring: Arc<SpscRing>,
    sfx_ring: Arc<SpscRing>,
    render_format: AudioFormat,
    mix_format: windows::Win32::Media::Audio::WAVEFORMATEX,
    fanout: JoinHandle<()>,
    targets: Vec<TargetState>,
}

struct ProcessThread { slot: usize, stop: Arc<AtomicBool>, thread: JoinHandle<()> }

#[derive(Default)]
struct SourceThreads {
    process: Vec<ProcessThread>,
    mic_stop: Option<Arc<AtomicBool>>,
    mic: Option<JoinHandle<()>>,
    sfx_tx: Option<std::sync::mpsc::Sender<crate::audio::sfx::SfxCommand>>,
    sfx_stop: Option<Arc<AtomicBool>>,
    sfx: Option<JoinHandle<()>>,
}

pub struct AudioEngine {
    volume: Arc<AtomicU32>,
    mic_volume: Arc<AtomicU32>,
    sfx_volume: Arc<AtomicU32>,
    master: Arc<AtomicU32>,
    capture_peak: Arc<AtomicU32>,
    mic_peak: Arc<AtomicU32>,
    render_peak: Arc<AtomicU32>,
    running: Arc<AtomicBool>,
    status: Arc<Mutex<EngineStatus>>,
    output: Mutex<Option<OutputState>>,
    sources: Mutex<SourceThreads>,
}

impl Default for AudioEngine { fn default() -> Self { Self::new() } }

impl AudioEngine {
    pub fn new() -> Self {
        Self {
            volume: Arc::new(AtomicU32::new(1.0f32.to_bits())),
            mic_volume: Arc::new(AtomicU32::new(1.0f32.to_bits())),
            sfx_volume: Arc::new(AtomicU32::new(1.0f32.to_bits())),
            master: Arc::new(AtomicU32::new(1.0f32.to_bits())),
            capture_peak: Arc::new(AtomicU32::new(0)),
            mic_peak: Arc::new(AtomicU32::new(0)),
            render_peak: Arc::new(AtomicU32::new(0)),
            running: Arc::new(AtomicBool::new(false)),
            status: Arc::new(Mutex::new(EngineStatus::Idle)),
            output: Mutex::new(None),
            sources: Mutex::new(SourceThreads::default()),
        }
    }
    pub fn is_running(&self) -> bool { self.running.load(Ordering::Acquire) }
    pub fn status(&self) -> EngineStatus { self.status.lock().map(|g| g.clone()).unwrap_or(EngineStatus::Error("status lock".into())) }
    pub fn set_volume(&self, v: f32) { self.volume.store(v.clamp(0.0, 1.0).to_bits(), Ordering::Relaxed); }
    pub fn set_mic_volume(&self, v: f32) { self.mic_volume.store(v.clamp(0.0, 1.0).to_bits(), Ordering::Relaxed); }
    pub fn set_sfx_volume(&self, v: f32) { self.sfx_volume.store(v.clamp(0.0, 1.0).to_bits(), Ordering::Relaxed); }
    pub fn set_master_volume(&self, v: f32) { self.master.store(v.clamp(0.0, 1.0).to_bits(), Ordering::Relaxed); }
    pub fn sample_capture_peak(&self) -> f32 { crate::audio::peak::sample_and_decay(&self.capture_peak, 0.62) }
    pub fn sample_mic_peak(&self) -> f32 { crate::audio::peak::sample_and_decay(&self.mic_peak, 0.62) }
    pub fn sample_render_peak(&self) -> f32 { crate::audio::peak::sample_and_decay(&self.render_peak, 0.62) }

    /// 单输出兼容入口。
    pub fn open_output(&self, device_id: &str) -> Result<(), Error> { self.open_outputs(&[device_id.to_owned()]) }

    /// 打开多个输出；每个输出拥有独立 ring/render，fan-out 负责格式转换。
    pub fn open_outputs(&self, device_ids: &[String]) -> Result<(), Error> {
        self.stop();
        if device_ids.is_empty() { return Err(Error::RenderInit("no output devices")); }
        let _com = com::init_mta()?;
        let mut clients = Vec::with_capacity(device_ids.len());
        for id in device_ids { clients.push(open_render_device(id)?); }
        let canonical = clients[0].format;
        let mix_format = copy_wave(clients[0].mix_format());
        let capacity = ((canonical.sample_rate as usize).saturating_mul(canonical.channels as usize) / 2).max(8192);
        let process_rings: Vec<Arc<SpscRing>> = (0..4).map(|_| Arc::new(SpscRing::with_capacity_samples(capacity))).collect();
        let mic_ring = Arc::new(SpscRing::with_capacity_samples(capacity));
        let sfx_ring = Arc::new(SpscRing::with_capacity_samples(capacity));
        let stop = Arc::new(AtomicBool::new(false));
        let mut targets = Vec::new();
        for client in clients {
            let target = client.format;
            let target_capacity = ((target.sample_rate as usize).saturating_mul(target.channels as usize) / 2).max(8192);
            let target_process: Vec<Arc<SpscRing>> = (0..4).map(|_| Arc::new(SpscRing::with_capacity_samples(target_capacity))).collect();
            let target_mic = Arc::new(SpscRing::with_capacity_samples(target_capacity));
            let target_sfx = Arc::new(SpscRing::with_capacity_samples(target_capacity));
            let status = self.status.clone();
            let running = self.running.clone();
            let render = spawn_target_render(self, client, target_process.clone(), target_mic.clone(), target_sfx.clone(), stop.clone(), status, running)?;
            targets.push(TargetState { format: target, process_rings: target_process, mic_ring: target_mic, sfx_ring: target_sfx, render });
        }
        let fanout_stop = stop.clone();
        let process_source = process_rings.clone();
        let mic_source = mic_ring.clone();
        let sfx_source = sfx_ring.clone();
        let fanout_targets = targets.iter().map(|t| FanoutTarget::new(t, canonical)).collect::<Vec<_>>();
        let fanout = thread::Builder::new().name("audio-fanout".into()).spawn(move || run_fanout(process_source, mic_source, sfx_source, fanout_targets, fanout_stop, canonical)).map_err(|_| Error::RenderInit("spawn output fanout"))?;
        let (sfx_tx, sfx_rx) = crate::audio::sfx::channel();
        let sfx_stop = Arc::new(AtomicBool::new(false));
        let sfx_stop_thread = sfx_stop.clone();
        let sfx_ring_thread = sfx_ring.clone();
        let sfx_thread = thread::Builder::new().name("sfx-mixer".into()).spawn(move || crate::audio::sfx::run_sfx_loop(sfx_ring_thread, canonical, sfx_stop_thread, sfx_rx)).map_err(|_| Error::RenderInit("spawn sfx thread"))?;
        if let Ok(mut output) = self.output.lock() { *output = Some(OutputState { stop, process_rings, mic_ring, sfx_ring, render_format: canonical, mix_format, fanout, targets }); }
        if let Ok(mut sources) = self.sources.lock() { sources.sfx_tx = Some(sfx_tx); sources.sfx_stop = Some(sfx_stop); sources.sfx = Some(sfx_thread); }
        self.set_status(EngineStatus::Running); self.running.store(true, Ordering::Release); Ok(())
    }

    pub fn set_process_source(&self, pid: u32) -> Result<(), Error> { self.stop_process_source(); self.add_process_source(pid, 0) }
    pub fn add_process_source(&self, pid: u32, slot: usize) -> Result<(), Error> {
        let _com = com::init_mta()?;
        let (rings, _, format, mix) = self.output_routing()?;
        let ring = rings.get(slot).cloned().ok_or(Error::CaptureInit("process source slots full"))?;
        let capture = open_process_capture(pid, Some(&mix))?;
        let converter = Converter::new(capture.format, AudioFormat { sample_rate: format.sample_rate, channels: format.channels, kind: SampleKind::F32 });
        let stop = Arc::new(AtomicBool::new(false)); let thread_stop = stop.clone(); let status = self.status.clone(); let peak = self.capture_peak.clone();
        let thread = thread::Builder::new().name(format!("wasapi-capture-{slot}")).spawn(move || { let _com = com::init_mta(); run_capture_loop(capture, ring, converter, thread_stop, peak, |err| { let mapped = match err { Error::ProcessNotFound(_) => EngineStatus::ProcessExited, Error::DeviceNotFound => EngineStatus::DeviceGone, other => EngineStatus::Error(other.user_message()) }; if let Ok(mut g)=status.lock(){*g=mapped;} }); }).map_err(|_| Error::CaptureInit("spawn capture thread"))?;
        if let Ok(mut sources) = self.sources.lock() { sources.process.push(ProcessThread { slot, stop, thread }); }
        Ok(())
    }
    pub fn set_mic_source(&self, mic_id: Option<&str>) -> Result<(), Error> { let _com = com::init_mta()?; self.stop_mic_source(); let Some(id)=mic_id else{return Ok(());}; let (_, ring, format, mix)=self.output_routing()?; let mic=open_mic_capture(id,Some(&mix))?; let conv=Converter::new(mic.format,AudioFormat{sample_rate:format.sample_rate,channels:format.channels,kind:SampleKind::F32}); let stop=Arc::new(AtomicBool::new(false)); let thread_stop=stop.clone(); let peak=self.mic_peak.clone(); let thread=thread::Builder::new().name("wasapi-mic".into()).spawn(move||{let _com=com::init_mta();run_capture_loop(mic,ring,conv,thread_stop,peak, |_|{});}).map_err(|_|Error::CaptureInit("spawn mic thread"))?; if let Ok(mut s)=self.sources.lock(){s.mic_stop=Some(stop);s.mic=Some(thread);} Ok(()) }
    pub fn start(&self, pid: u32, device_id: &str, mic_id: Option<&str>) -> Result<(), Error> { self.open_output(device_id)?; self.set_process_source(pid)?; self.set_mic_source(mic_id)?; Ok(()) }
    pub fn sfx_sender(&self) -> Result<std::sync::mpsc::Sender<crate::audio::sfx::SfxCommand>, Error> { self.sources.lock().map_err(|_|Error::RenderInit("source lock poisoned"))?.sfx_tx.clone().ok_or(Error::RenderInit("sfx is closed")) }
    pub fn stop(&self) { self.stop_process_source();self.stop_mic_source();self.stop_sfx_source(); if let Some(o)=self.output.lock().ok().and_then(|mut g|g.take()){o.stop.store(true,Ordering::Release);let _=o.fanout.join();for t in o.targets{let _=t.render.join();}} self.running.store(false,Ordering::Release);if matches!(self.status(),EngineStatus::Running){self.set_status(EngineStatus::Idle);} }
    fn output_routing(&self)->Result<(Vec<Arc<SpscRing>>,Arc<SpscRing>,AudioFormat,windows::Win32::Media::Audio::WAVEFORMATEX),Error>{let o=self.output.lock().map_err(|_|Error::RenderInit("output lock poisoned"))?;let o=o.as_ref().ok_or(Error::RenderInit("output is closed"))?;Ok((o.process_rings.clone(),o.mic_ring.clone(),o.render_format,copy_wave(&o.mix_format)))}
    fn stop_process_source(&self){let v=self.sources.lock().ok().map(|mut s|std::mem::take(&mut s.process)).unwrap_or_default();for p in v{p.stop.store(true,Ordering::Release);let _=p.thread.join();}}
    fn stop_mic_source(&self){let(s,t)=self.sources.lock().ok().map(|mut s|(s.mic_stop.take(),s.mic.take())).unwrap_or((None,None));if let Some(s)=s{s.store(true,Ordering::Release)}if let Some(t)=t{let _=t.join();}}
    fn stop_sfx_source(&self){let(s,t)=self.sources.lock().ok().map(|mut s|(s.sfx_stop.take(),s.sfx.take())).unwrap_or((None,None));if let Some(s)=s{s.store(true,Ordering::Release)}if let Some(t)=t{let _=t.join();}if let Ok(mut s)=self.sources.lock(){s.sfx_tx=None;}}
    fn set_status(&self,s:EngineStatus){if let Ok(mut g)=self.status.lock(){*g=s;}}
}

struct FanoutTarget { process_rings: Vec<Arc<SpscRing>>, mic_ring: Arc<SpscRing>, sfx_ring: Arc<SpscRing>, converters: Vec<Converter>, mic_converter: Converter, sfx_converter: Converter }
impl FanoutTarget { fn new(target:&TargetState, src:AudioFormat)->Self{let dst=target.format;Self{process_rings:target.process_rings.clone(),mic_ring:target.mic_ring.clone(),sfx_ring:target.sfx_ring.clone(),converters:(0..target.process_rings.len()).map(|_|Converter::new(src,dst)).collect(),mic_converter:Converter::new(src,dst),sfx_converter:Converter::new(src,dst)}}}
fn spawn_target_render(engine:&AudioEngine,client:crate::audio::render::RenderClient,process:Vec<Arc<SpscRing>>,mic:Arc<SpscRing>,sfx:Arc<SpscRing>,stop:Arc<AtomicBool>,status:Arc<Mutex<EngineStatus>>,running:Arc<AtomicBool>)->Result<JoinHandle<()>,Error>{let volume=engine.volume.clone();let mic_volume=engine.mic_volume.clone();let sfx_volume=engine.sfx_volume.clone();let master=engine.master.clone();let peak=engine.render_peak.clone();thread::Builder::new().name("wasapi-render-target".into()).spawn(move||{let _com=com::init_mta();run_render_loop(client,process,Some(mic),Some(sfx),volume,mic_volume,sfx_volume,master,stop,peak,move|e|{let mapped=match e{Error::DeviceNotFound=>EngineStatus::DeviceGone,other=>EngineStatus::Error(other.user_message())};if let Ok(mut g)=status.lock(){*g=mapped;}running.store(false,Ordering::Release);});}).map_err(|_|Error::RenderInit("spawn output target"))}
fn run_fanout(source:Vec<Arc<SpscRing>>,mic:Arc<SpscRing>,sfx:Arc<SpscRing>,mut targets:Vec<FanoutTarget>,stop:Arc<AtomicBool>,format:AudioFormat){let frames=(format.sample_rate as usize/100).max(64);let samples=frames*format.channels as usize;let mut blocks=vec![vec![0.0; samples];source.len()];let mut mic_block=vec![0.0;samples];let mut sfx_block=vec![0.0;samples];while !stop.load(Ordering::Acquire){for(r,b)in source.iter().zip(blocks.iter_mut()){let n=r.pop(b);b[n..].fill(0.0);}let mn=mic.pop(&mut mic_block);mic_block[mn..].fill(0.0);let sn=sfx.pop(&mut sfx_block);sfx_block[sn..].fill(0.0);for t in &mut targets{for(i,b)in blocks.iter().enumerate(){let mut out=Vec::new();t.converters[i].convert_f32(b,frames,&mut out);t.process_rings[i].push_latest(&out);}let mut out=Vec::new();t.mic_converter.convert_f32(&mic_block,frames,&mut out);t.mic_ring.push_latest(&out);out.clear();t.sfx_converter.convert_f32(&sfx_block,frames,&mut out);t.sfx_ring.push_latest(&out);}thread::sleep(Duration::from_millis(10));}}
impl Drop for AudioEngine { fn drop(&mut self){self.stop();}}
fn copy_wave(src:&windows::Win32::Media::Audio::WAVEFORMATEX)->windows::Win32::Media::Audio::WAVEFORMATEX{*src}

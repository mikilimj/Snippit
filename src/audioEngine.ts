//! Chunked preview-audio engine. Plays every audio track of the current clip
//! through ONE shared AudioContext by fetching ffmpeg-decoded PCM chunks from
//! the Rust side and scheduling them as AudioBufferSourceNodes. No <audio>
//! elements are involved: on WebKitGTK each media element keeps its own (muted)
//! system audio sink alive even when captured into Web Audio, so element-based
//! playback made every track show up as a separate stream in the OS mixer.
//! With this engine the app registers exactly one output stream — the context's.

import { audioChunk } from "./api";
import {
  CHUNK_SECONDS,
  chunkIndexAt,
  chunkSchedule,
  chunkWindow,
  engineMediaTime,
  needsResync,
} from "./audioChunks";

/** Decoded chunks kept per track: current, neighbours, and a little history. */
const CACHE_CHUNKS_PER_TRACK = 4;

export class ChunkedAudioEngine {
  private readonly ctx: AudioContext;
  private readonly trackPaths: string[];
  private readonly totalDur: number;
  private readonly gains: GainNode[];

  private buffers = new Map<string, AudioBuffer>();
  private inflight = new Map<string, Promise<AudioBuffer | null>>();
  private scheduled = new Map<string, AudioBufferSourceNode>();

  private anchorMedia = 0;
  private anchorCtx = 0;
  private playing = false;
  private disposed = false;
  /** Bumped whenever scheduled sources are torn down; stale async schedules bail. */
  private epoch = 0;

  constructor(ctx: AudioContext, trackPaths: string[], totalDur: number, initialGains: number[]) {
    this.ctx = ctx;
    this.trackPaths = trackPaths;
    this.totalDur = totalDur;
    this.gains = trackPaths.map((_, i) => {
      const g = ctx.createGain();
      g.gain.value = initialGains[i] ?? 1;
      g.connect(ctx.destination);
      return g;
    });
  }

  /** Live per-track volume (0 when the track is muted), same as the old sliders. */
  setTrackGain(i: number, value: number) {
    const g = this.gains[i];
    if (g) g.gain.value = value;
  }

  /**
   * The media-clock position the engine is currently rendering, or null when
   * not playing. Used to land a <video> source swap exactly where the (still
   * uninterrupted) audio is.
   */
  currentMediaTime(): number | null {
    if (!this.playing || this.disposed) return null;
    return engineMediaTime(this.ctx.currentTime, this.anchorMedia, this.anchorCtx);
  }

  /** (Re)start playback anchored at media time `at`. Also serves as resync. */
  play(at: number) {
    if (this.disposed) return;
    this.stopSources();
    this.playing = true;
    this.anchorMedia = at;
    this.anchorCtx = this.ctx.currentTime;
    void this.ctx.resume().catch(() => {});
    this.ensureWindow(at);
  }

  /**
   * Halt output instantly by stopping every scheduled source. The context is
   * deliberately left running: on WebKitGTK suspend()/resume() tears down and
   * recreates the system sink, so each pause/play cycle would register a brand
   * new OS audio stream. Stopped sources are sample-accurate silence anyway.
   */
  pause() {
    if (this.disposed) return;
    this.playing = false;
    this.stopSources();
  }

  seek(at: number) {
    if (this.playing) this.play(at);
  }

  /**
   * Periodic keep-in-sync check driven by the video's timeupdate. Re-anchors on
   * drift or stall, and tops up the schedule as the playhead approaches a chunk
   * boundary. Deliberately cheap: no allocation-heavy work in the common case.
   */
  tick(videoTime: number, videoPaused: boolean) {
    if (this.disposed) return;
    if (videoPaused) {
      if (this.playing) this.pause();
      return;
    }
    if (!this.playing) {
      this.play(videoTime);
      return;
    }
    const engineTime = engineMediaTime(this.ctx.currentTime, this.anchorMedia, this.anchorCtx);
    if (needsResync(videoTime, engineTime)) {
      this.play(videoTime);
      return;
    }
    this.ensureWindow(videoTime);
  }

  dispose() {
    this.disposed = true;
    this.stopSources();
    this.gains.forEach(g => g.disconnect());
    this.buffers.clear();
    this.inflight.clear();
  }

  private stopSources() {
    this.epoch++;
    this.scheduled.forEach(src => {
      src.onended = null;
      try {
        src.stop();
      } catch {
        // never started or already stopped — nothing to do
      }
      src.disconnect();
    });
    this.scheduled.clear();
  }

  private ensureWindow(t: number) {
    for (const idx of chunkWindow(t, this.totalDur)) {
      for (let track = 0; track < this.trackPaths.length; track++) {
        void this.scheduleChunk(track, idx);
      }
    }
  }

  private async scheduleChunk(track: number, idx: number) {
    const key = `${track}:${idx}`;
    if (this.scheduled.has(key)) return;
    const epoch = this.epoch;
    const buf = await this.getBuffer(track, idx);
    // The world may have moved on while the chunk was fetched/decoded.
    if (!buf || this.disposed || !this.playing || epoch !== this.epoch || this.scheduled.has(key)) {
      return;
    }
    const spec = chunkSchedule(
      idx,
      this.anchorMedia,
      this.anchorCtx,
      this.ctx.currentTime,
      this.totalDur
    );
    if (!spec) return;
    const src = this.ctx.createBufferSource();
    src.buffer = buf;
    src.connect(this.gains[track]);
    src.onended = () => {
      if (this.scheduled.get(key) === src) this.scheduled.delete(key);
      src.disconnect();
    };
    src.start(spec.when, spec.offset, spec.duration);
    this.scheduled.set(key, src);
  }

  private getBuffer(track: number, idx: number): Promise<AudioBuffer | null> {
    const key = `${track}:${idx}`;
    const cached = this.buffers.get(key);
    if (cached) return Promise.resolve(cached);
    const pending = this.inflight.get(key);
    if (pending) return pending;

    const p = audioChunk(this.trackPaths[track], idx * CHUNK_SECONDS, CHUNK_SECONDS)
      .then(bytes => this.ctx.decodeAudioData(bytes))
      .then(buf => {
        if (this.disposed) return null;
        this.buffers.set(key, buf);
        this.evictFarChunks(track);
        return buf;
      })
      .catch(err => {
        console.error(`audio chunk ${key} failed:`, err);
        return null;
      })
      .finally(() => {
        this.inflight.delete(key);
      });
    this.inflight.set(key, p);
    return p;
  }

  /** Drop cached chunks far from the playhead so memory stays bounded. */
  private evictFarChunks(track: number) {
    const cur = chunkIndexAt(
      engineMediaTime(this.ctx.currentTime, this.anchorMedia, this.anchorCtx)
    );
    const prefix = `${track}:`;
    const owned = [...this.buffers.keys()].filter(k => k.startsWith(prefix));
    if (owned.length <= CACHE_CHUNKS_PER_TRACK) return;
    owned
      .sort(
        (a, b) =>
          Math.abs(Number(b.slice(prefix.length)) - cur) -
          Math.abs(Number(a.slice(prefix.length)) - cur)
      )
      .slice(0, owned.length - CACHE_CHUNKS_PER_TRACK)
      .forEach(k => this.buffers.delete(k));
  }
}

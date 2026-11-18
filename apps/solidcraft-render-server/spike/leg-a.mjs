// Leg-A harness: plays the host (PROTOCOL.md) for one render server, with no webview, for M2 over the
// wire and M3's server + transport share of the latency. Node 18+, no dependencies.
//
//   node leg-a.mjs --bin PATH [--size 1280x800] [--dpr 2] [--adapter auto] [--open FILE]
//                  [--case orbit|edit|click|typing|idle|none, or several: orbit,click,typing] [--button secondary] [--moves 60]
//                  [--hz 120] [--idle-ms 60000] [--clicks 5] [--click-handle toolbar:solid.extrude] [--rounds 3]
//                  [--type-handle ribbon:search] [--out DIR]
//                  [--policy REASON] (read-only from the start: `policy {readOnly: REASON}`)
//                  [--lang zh] [--answer-dir DIR] (a save ask is answered DIR/export-<ask>/<name>, as the host's staging place)
//
// Cases beyond M2/M3: rename (select the first body's browser row, F2, 测试 composed by IME, Enter: the body is renamed);
// export (command search, 导出 composed by IME, Enter: the save ask, the file written where it was answered, and no
// status line or message naming that place; needs --answer-dir and --out); brand (command search for upstream's name
// and its help, report and update entries, the About window, and diagnostics.report_issue over the control channel, which
// must refuse: what each shows, as handles and screenshots in --out).
//
// M3 (click, typing): each measured input's latency is input sent → first frame whose inputSeq ≥ its seq, split into the
// server's own stages of that frame (pass, paint, readback, diff, encode) and the rest (waiting for the pass to start —
// the pass in flight, the 60/s pacing — plus loopback and this process). typing: keys 2 and 5 (text + key, as the pane
// sends a key typed on the canvas), then 测试 composed as a pinyin IME does (preedit c, ce, commit 测; s, sh, shi, commit 试).
// idle: frames sent and the render server's CPU share (/proc, Linux) over the window.
//
// Prints one JSON summary on stdout: startup times, frames, bytes, per-stage medians from frame.timing,
// input→frame latency (p50/p95, by frame.inputSeq), frames/s during the drag, the camera and the
// design revision before/after (orbit), frames sent while idle (idle). Frames are acked at once.
import { spawn, execFileSync } from 'node:child_process'
import crypto from 'node:crypto'
import fs from 'node:fs'
import net from 'node:net'
import path from 'node:path'

const opt = { size: '1280x800', dpr: '1', adapter: 'auto', case: 'none', button: 'secondary', moves: '60', hz: '120', 'idle-ms': '60000',
  clicks: '5', 'click-handle': 'toolbar:solid.extrude', rounds: '3', 'type-handle': 'ribbon:search' }
const argv = process.argv.slice(2)
for (let i = 0; i < argv.length; i += 2) opt[argv[i].replace(/^--/, '')] = argv[i + 1]
if (!opt.bin) {
  console.error('usage: node leg-a.mjs --bin PATH [...] (see the header)')
  process.exit(2)
}
const [W, H] = opt.size.split('x').map(Number)
const renderToken = crypto.randomBytes(32).toString('hex')
const controlToken = crypto.randomBytes(32).toString('hex')
const t0 = performance.now()
const now = () => performance.now() - t0
const marks = {}
const frames = []
const sentAt = new Map()
const notes = []
const measured = [] // { group, label, seq }: the inputs M3 times
let sock, child, seq = 0, control

// ---- leg A framing: u32 LE envelope length, u32 LE header length, JSON header, payload ----
function send(obj) {
  const header = Buffer.from(JSON.stringify(obj))
  const out = Buffer.alloc(8 + header.length)
  out.writeUInt32LE(4 + header.length, 0)
  out.writeUInt32LE(header.length, 4)
  header.copy(out, 8)
  sock.write(out)
}
function reader(onMessage) {
  let buf = Buffer.alloc(0)
  return (chunk) => {
    buf = Buffer.concat([buf, chunk])
    while (buf.length >= 4) {
      const len = buf.readUInt32LE(0)
      if (buf.length < 4 + len) return
      const env = buf.subarray(4, 4 + len)
      const hlen = env.readUInt32LE(0)
      onMessage(JSON.parse(env.subarray(4, 4 + hlen).toString('utf8')), env.subarray(4 + hlen))
      buf = buf.subarray(4 + len)
    }
  }
}

// ---- the control channel (JSON lines, token on the first request) ----
function connectControl(port) {
  const s = net.connect(port, '127.0.0.1')
  const waiting = new Map()
  let id = 0, buf = '', first = true
  s.on('data', (d) => {
    buf += d
    for (let nl; (nl = buf.indexOf('\n')) >= 0; buf = buf.slice(nl + 1)) {
      const r = JSON.parse(buf.slice(0, nl))
      waiting.get(r.id)?.(r)
      waiting.delete(r.id)
    }
  })
  return (method, params = {}) =>
    new Promise((resolve) => {
      const req = { id: ++id, method, params }
      if (first) (req.token = controlToken), (first = false)
      waiting.set(req.id, resolve)
      s.write(JSON.stringify(req) + '\n')
    })
}

const sleep = (ms) => new Promise((r) => setTimeout(r, ms))
const mods = { alt: false, ctrl: false, shift: false, meta: false }
function input(events, group, label) {
  seq += 1
  sentAt.set(seq, now())
  send({ k: 'input', seq, events })
  if (group) measured.push({ group, label, seq })
}
const key = (k, physical, pressed) => ({ e: 'key', key: k, physical, pressed, repeat: false, mods })
async function tap(x, y, group, label) {
  input([{ e: 'move', x, y }])
  await sleep(300)
  input([{ e: 'button', x, y, button: 'primary', pressed: true, mods }])
  await sleep(80)
  input([{ e: 'button', x, y, button: 'primary', pressed: false, mods }], group, label)
}
async function escape() {
  input([key('Escape', 'Escape', true)])
  await sleep(30)
  input([key('Escape', 'Escape', false)])
  await sleep(400)
}
async function handle(name, prefix) {
  const all = (await control('ui.handles', { prefix })).result ?? []
  const h = all.find((x) => x.name === name) ?? all[0]
  if (!h) throw new Error(`no handle ${name} (prefix ${prefix})`)
  if (h.name !== name) notes.push(`handle ${name} missing, used ${h.name}`)
  return h
}
/** CPU ms (utime + stime) and RSS of the render server, from /proc (Linux); null elsewhere. */
let clkTck = null
function procSample() {
  try {
    clkTck ??= Number(execFileSync('getconf', ['CLK_TCK'], { encoding: 'utf8' }).trim()) || 100
    const stat = fs.readFileSync(`/proc/${child.pid}/stat`, 'utf8')
    const f = stat.slice(stat.lastIndexOf(')') + 2).split(' ')
    const rssKb = Number(/VmRSS:\s+(\d+)/.exec(fs.readFileSync(`/proc/${child.pid}/status`, 'utf8'))?.[1])
    return { at: now(), cpuMs: ((Number(f[11]) + Number(f[12])) * 1000) / clkTck, rssMb: Math.round(rssKb / 102.4) / 10 }
  } catch {
    return null
  }
}

function onMessage(h, payload) {
  switch (h.k) {
    case 'hello': {
      marks.hello = now()
      const ok = h.v === 1 && crypto.timingSafeEqual(Buffer.from(h.token), Buffer.from(renderToken))
      notes.push(`hello: ${h.backend} ${h.adapter} software=${h.software} token=${ok}`)
      if (!ok) return send({ k: 'error', code: 'token', message: 'wrong token' })
      send({ k: 'policy', readOnly: opt.policy ?? null })
      send({ k: 'resize', w: W, h: H, dpr: Number(opt.dpr) })
      send({ k: 'visible', visible: true })
      return
    }
    case 'ready':
      marks.ready = now()
      marks.samples = h.samples
      control = connectControl(h.controlPort)
      runCase().catch((e) => notes.push(`case failed: ${e}`)).finally(stop)
      return
    case 'frame':
      marks.firstFrame ??= now()
      frames.push({ seq: h.seq, inputSeq: h.inputSeq, at: now(), bytes: payload.length, full: h.full, rects: h.rects.length, codecs: h.rects.map((r) => r.codec), timing: h.timing })
      send({ k: 'ack', seq: h.seq, decodeMs: 0 })
      if (opt.out && (h.seq === 1 || h.full)) {
        fs.mkdirSync(opt.out, { recursive: true })
        h.rects.forEach((r, i) => fs.writeFileSync(path.join(opt.out, `f${h.seq}-r${i}-${r.x}x${r.y}-${r.w}x${r.h}.${r.codec}`), payload.subarray(r.offset, r.offset + r.length)))
      }
      return
    case 'ask_save':
      if (opt['answer-dir']) {
        const name = String(h.name).split(/[\\/]/).pop() || 'export'
        const dir = path.resolve(opt['answer-dir'], `export-${h.ask}`)
        fs.mkdirSync(dir, { recursive: true })
        const place = path.join(dir, name)
        marks.saves ??= []
        marks.saves.push({ ask: h.ask, name: h.name, exts: h.exts, place })
        notes.push(`ask_save ${JSON.stringify(h)}: answered ${place}`)
        return send({ k: 'answer', ask: h.ask, path: place })
      }
    // falls through: answered null
    case 'ask_open':
      notes.push(`${h.k} ${JSON.stringify(h)}: answered null`)
      return send({ k: 'answer', ask: h.ask, path: null })
    case 'ask_done': {
      const save = marks.saves?.find((x) => x.ask === h.ask)
      if (save) (save.ok = h.ok), (save.bytes = fs.existsSync(save.place) ? fs.statSync(save.place).size : null)
      notes.push(`ask_done ${JSON.stringify(h)}`)
      return
    }
    case 'ime':
      marks.ime = h.ime
      notes.push(`ime ${JSON.stringify(h.ime)}`)
      return
    case 'bye':
      marks.bye = now()
      notes.push(`bye ${h.why} ${h.detail ?? ''}`)
      return
    default:
      notes.push(`${h.k} ${JSON.stringify(h).slice(0, 200)}`)
  }
}

async function openDesign() {
  if (!opt.open) return
  const t = now()
  notes.push(`ui.file: ${JSON.stringify((await control('ui.file', { path: path.resolve(opt.open) })).ok)}`)
  for (let i = 0; i < 600; i++) {
    const r = await control('ui.inspect')
    if (r.ok && !r.result?.file_busy) break
    await sleep(100)
  }
  marks.openMs = now() - t
  await control('ui.view', { view: 'fit' })
  await sleep(1000)
}

async function runCase() {
  await openDesign()
  for (const c of opt.case.split(',')) await runOne(c)
  marks.rssEnd = procSample()?.rssMb
}

async function runOne(name) {
  if (name === 'orbit') {
    const before = { camera: (await control('ui.camera')).result, revision: (await control('document.revision')).result }
    const [cx, cy] = [W / 2, H / 2]
    const n = Number(opt.moves)
    const gap = 1000 / Number(opt.hz)
    input([{ e: 'move', x: cx, y: cy }])
    await sleep(50)
    marks.dragFrom = now()
    marks.dragSeqFrom = seq + 1
    input([{ e: 'button', x: cx, y: cy, button: opt.button, pressed: true, mods }])
    for (let i = 1; i <= n; i++) {
      await sleep(gap)
      input([{ e: 'move', x: cx + (200 * i) / n, y: cy }])
    }
    input([{ e: 'button', x: cx + 200, y: cy, button: opt.button, pressed: false, mods }])
    marks.dragTo = now()
    marks.dragSeqTo = seq
    await sleep(500)
    const after = { camera: (await control('ui.camera')).result, revision: (await control('document.revision')).result }
    marks.orbit = { before, after }
  } else if (name === 'edit') {
    // Select the first body by its browser row (its handle from the running UI), then Delete:
    // the revision must move, or stay with --policy (and the body count with it).
    const state = async () => ({
      revision: (await control('document.revision')).result,
      bodies: (await control('document.inspect', { bodies: 'names', timeline: 'summary' })).result,
    })
    const before = await state()
    const rows = (await control('ui.handles', { prefix: 'row:' })).result ?? []
    // `bodies: "names"` lists plain names (docs/control-protocol.md, document.inspect).
    const first = before.bodies?.bodies?.[0]
    const name = typeof first === 'string' ? first : first?.name
    const row = rows.find((r) => r.name === `row:${name}`)
    if (!row) throw new Error(`no browser row for body ${name}`)
    input([{ e: 'focus', focused: true }, { e: 'move', x: row.x - 40, y: row.y }])
    await sleep(100)
    input([{ e: 'button', x: row.x - 40, y: row.y, button: 'primary', pressed: true, mods }])
    input([{ e: 'button', x: row.x - 40, y: row.y, button: 'primary', pressed: false, mods }])
    await sleep(500)
    const selected = (await control('document.inspect', { bodies: false, timeline: false })).result?.selection
    input([key('Delete', 'Delete', true)])
    input([key('Delete', 'Delete', false)])
    await sleep(3000)
    const after = await state()
    const status = (await control('ui.inspect')).result?.status
    const count = (s) => s.bodies?.bodies?.length
    const features = (s) => s.bodies?.timeline?.length
    marks.edit = {
      row,
      selected,
      before: { revision: before.revision, bodies: count(before), features: features(before) },
      after: { revision: after.revision, bodies: count(after), features: features(after) },
      status,
    }
  } else if (name === 'click') {
    // A ribbon button, timed from its release (a toolbar button acts on release), then Escape.
    const b = await handle(opt['click-handle'], 'toolbar:')
    marks.click = { handle: b }
    for (let i = 0; i < Number(opt.clicks); i++) {
      await tap(b.x, b.y, 'click', `click ${i + 1}`)
      await sleep(600)
      await escape()
    }
  } else if (name === 'typing') {
    const f = await handle(opt['type-handle'], '')
    marks.typing = { handle: f, rounds: [] }
    for (let r = 0; r < Number(opt.rounds); r++) {
      await tap(f.x, f.y)
      await sleep(300)
      for (const [k, code] of [['2', 'Digit2'], ['5', 'Digit5']]) {
        input([{ e: 'text', text: k }, key(k, code, true)], 'typing', `key ${k}`)
        await sleep(20)
        input([key(k, code, false)])
        await sleep(130)
      }
      for (const [pre, word] of [[['c', 'ce'], '测'], [['s', 'sh', 'shi'], '试']]) {
        for (const text of pre) {
          input([{ e: 'preedit', text, range: [text.length, text.length] }], 'typing', `preedit ${text}`)
          await sleep(150)
        }
        input([{ e: 'commit', text: word }], 'typing', `commit ${word}`)
        await sleep(150)
      }
      marks.typing.rounds.push({ imeOpen: marks.ime ?? null })
      await escape()
    }
  } else if (name === 'rename') {
    // The first body's browser row, F2, then 测试 as a pinyin IME composes it, and Enter.
    const names = async () => ((await control('document.inspect', { bodies: 'names', timeline: false })).result?.bodies ?? []).map((b) => (typeof b === 'string' ? b : b.name))
    const before = await names()
    const row = (await control('ui.handles', { prefix: 'row:' })).result?.find((r) => r.name === `row:${before[0]}`)
    if (!row) throw new Error(`no browser row for body ${before[0]}`)
    input([{ e: 'focus', focused: true }, { e: 'move', x: row.x - 40, y: row.y }])
    await sleep(100)
    input([{ e: 'button', x: row.x - 40, y: row.y, button: 'primary', pressed: true, mods }])
    input([{ e: 'button', x: row.x - 40, y: row.y, button: 'primary', pressed: false, mods }])
    await sleep(400)
    input([key('F2', 'F2', true)])
    input([key('F2', 'F2', false)])
    await sleep(400)
    marks.ime = undefined
    for (const [text, range] of [['ce', [2, 2]], ['测', [0, 1]]]) {
      input([{ e: 'preedit', text, range }])
      await sleep(200)
    }
    input([{ e: 'commit', text: '测试' }])
    await sleep(200)
    input([key('Enter', 'Enter', true)])
    input([key('Enter', 'Enter', false)])
    await sleep(1500)
    marks.rename = { row: row.name, before: before[0], after: (await names())[0], ime: marks.ime ?? null }
  } else if (name === 'export') {
    // Command search, 导出 composed, Enter: Export itself asks where to save; the answer is a staging place.
    const f = await handle(opt['type-handle'], '')
    await tap(f.x, f.y)
    await sleep(400)
    input([{ e: 'preedit', text: 'daochu', range: [6, 6] }])
    await sleep(200)
    input([{ e: 'commit', text: '导出' }])
    await sleep(800)
    const rows = ((await control('ui.handles', { prefix: 'search:command:' })).result ?? []).map((r) => r.name)
    input([key('Enter', 'Enter', true)])
    input([key('Enter', 'Enter', false)])
    for (let i = 0; i < 100 && !marks.saves?.some((x) => x.ok !== undefined); i++) await sleep(100)
    await sleep(1500)
    const places = (marks.saves ?? []).map((x) => x.place)
    const named = (text) => places.some((p) => String(text).includes(p))
    const status = (await control('ui.inspect')).result?.status ?? null
    const messages = (await control('ui.messages')).result?.messages ?? []
    if (opt.out) await control('ui.screenshot', { path: path.resolve(opt.out, 'export-after.png') })
    marks.export = { searchRows: rows.slice(0, 8), saves: marks.saves ?? [], status, statusNamesPlace: named(status?.text ?? ''), messagesNamingPlace: messages.filter((m) => named(m.text)), lastMessages: messages.slice(-3) }
  } else if (name === 'brand') {
    const queries = ['SolidCraft', 'solidcraft', '关于', '帮助', '报告', '问题', '更新', 'Discord', 'Ko-fi', '文档', '设置', '主题', '助手', 'AI', '插件', 'About', 'Report', 'Help']
    marks.brand = { search: {} }
    for (const [i, text] of queries.entries()) {
      await control('ui.palette', { text })
      await sleep(700)
      marks.brand.search[text] = ((await control('ui.handles', { prefix: 'search:command:' })).result ?? []).map((r) => r.name.replace('search:command:', ''))
      if (opt.out) await control('ui.screenshot', { path: path.resolve(opt.out, `brand-search-${i}.png`) })
      await control('ui.palette', { open: false })
      await sleep(200)
    }
    marks.brand.help = {}
    for (const item of ['report', 'docs', 'updates', 'crashes', 'about']) {
      const r = await control('ui.help', { item })
      await sleep(700)
      const shown = ((await control('ui.handles', { prefix: '' })).result ?? []).map((x) => x.name).filter((n) => /^(report|issue|crash|update|about)/.test(n))
      marks.brand.help[item] = { ok: r.ok, error: r.error ?? null, shown }
    }
    // The model's way in: the command itself over the control channel must refuse.
    const asked = await control('engine.execute', { command: 'diagnostics.report_issue', params: { title: 'probe' } })
    marks.brand.reportIssue = { ok: asked.ok, error: asked.error ?? null }
    // Upstream's Assistant: refused as a command and through its own control, whoever asks.
    const assistant = await control('engine.execute', { command: 'ui.assistant', params: {} })
    const agent = await control('ui.agent_chat', { connect: true })
    marks.brand.assistant = { ok: assistant.ok, error: assistant.error ?? null, agentChat: { ok: agent.ok, error: agent.error ?? null } }
    if (opt.out) await control('ui.screenshot', { path: path.resolve(opt.out, 'brand-about.png') })
    marks.brand.aboutHandles = ((await control('ui.handles', { prefix: 'about:' })).result ?? []).map((r) => r.name)
    await escape()
  } else if (name === 'idle') {
    await sleep(2000)
    const from = frames.length
    const before = procSample()
    const series = [before]
    for (let left = Number(opt['idle-ms']); left > 0; left -= 5000) {
      await sleep(Math.min(5000, left))
      series.push(procSample())
    }
    const after = series.at(-1)
    marks.idleFrames = frames.length - from
    marks.idle = {
      seconds: before && after ? (after.at - before.at) / 1000 : null,
      frames: frames.length - from,
      framesDetail: frames.slice(from).map((f) => ({ at: Math.round(f.at), inputSeq: f.inputSeq, bytes: f.bytes, codecs: f.codecs })),
      cpuPct: before && after ? Math.round(((after.cpuMs - before.cpuMs) / (after.at - before.at)) * 10000) / 100 : null,
      cpuMs: before && after ? after.cpuMs - before.cpuMs : null,
      rssMb: after?.rssMb,
      // per 5 s slice: frames received and the render server's CPU share in it
      slices: series.slice(1).map((b, i) => {
        const a = series[i]
        if (!a || !b) return null
        const n = frames.slice(from).filter((f) => f.at > a.at && f.at <= b.at).length
        return { fromS: r1((a.at - before.at) / 1000), frames: n, cpuPct: Math.round(((b.cpuMs - a.cpuMs) / (b.at - a.at)) * 10000) / 100 }
      }),
    }
  }
}

function stop() {
  send({ k: 'stop', why: 'harness' })
  setTimeout(() => {
    notes.push('no exit within 6 s of stop: killed')
    child.kill()
  }, 6000).unref()
}

const STAGES = ['pass', 'paint', 'readback', 'diff', 'encode']
const r1 = (x) => (x == null ? null : Math.round(x * 10) / 10)
/** One measured input: its frame, the latency, the frame's own stages and the rest. */
function timed(m) {
  const t = sentAt.get(m.seq)
  const f = frames.find((x) => x.inputSeq >= m.seq && x.at >= t)
  if (!f) return { ...m, missing: true }
  const latency = f.at - t
  const server = STAGES.reduce((a, k) => a + (f.timing[k] ?? 0), 0)
  return { ...m, frame: f.seq, latency: r1(latency), ...Object.fromEntries(STAGES.map((k) => [k, r1(f.timing[k])])), rest: r1(latency - server), bytes: f.bytes, codecs: f.codecs }
}
function group(rows) {
  const ok = rows.filter((r) => !r.missing)
  const p = (k, q) => r1(pct(ok.map((r) => r[k]), q))
  return {
    n: rows.length, missing: rows.length - ok.length,
    latencyMs: { p50: p('latency', 50), p95: p('latency', 95), max: r1(Math.max(...ok.map((r) => r.latency))) },
    stageMsP50: Object.fromEntries([...STAGES, 'rest'].map((k) => [k, p(k, 50)])),
    bytesP50: pct(ok.map((r) => r.bytes), 50),
    events: rows,
  }
}
/** Drag frames: stages, the frame-to-frame cycle and how much of it the stages do not account for. */
function dragStages(drag) {
  const rows = drag.map((f, i) => {
    const server = STAGES.reduce((a, k) => a + (f.timing[k] ?? 0), 0)
    const latency = f.at - sentAt.get(f.inputSeq)
    return { inputSeq: f.inputSeq, ...Object.fromEntries(STAGES.map((k) => [k, f.timing[k]])), server, latency, rest: latency - server, gap: i ? f.at - drag[i - 1].at : null, bytes: f.bytes, jpeg: f.codecs.filter((c) => c === 'jpeg').length, rgba: f.codecs.filter((c) => c === 'rgba').length }
  })
  const p = (k, q) => r1(pct(rows.map((r) => r[k]).filter((x) => x != null), q))
  return {
    stageMsP50: Object.fromEntries([...STAGES, 'server', 'rest', 'gap'].map((k) => [k, p(k, 50)])),
    stageMsP95: Object.fromEntries([...STAGES, 'server', 'rest', 'gap'].map((k) => [k, p(k, 95)])),
    bytesP50: pct(rows.map((r) => r.bytes), 50),
    rows: rows.map((r) => Object.fromEntries(Object.entries(r).map(([k, v]) => [k, typeof v === 'number' ? r1(v) : v]))),
  }
}
const pct = (xs, p) => (xs.length ? [...xs].sort((a, b) => a - b)[Math.min(xs.length - 1, Math.floor((p / 100) * xs.length))] : null)
function summary(code) {
  const stage = (k) => pct(frames.map((f) => f.timing[k]), 50)
  // Drag frames: the first frame carrying each input of the drag (press .. release). A later frame with the same inputSeq is
  // the UI repainting on its own after the input (an animation), not a response to it: counted apart as `after`.
  const inDrag = frames.filter((f) => marks.dragSeqFrom && f.inputSeq >= marks.dragSeqFrom && f.inputSeq <= marks.dragSeqTo && f.at >= marks.dragFrom)
  const drag = inDrag.filter((f, i) => i === 0 || f.inputSeq !== inDrag[i - 1].inputSeq)
  const latency = drag.map((f) => f.at - sentAt.get(f.inputSeq)).filter((x) => x >= 0)
  const span = drag.length > 1 ? (drag.at(-1).at - drag[0].at) / 1000 : 0
  return {
    exit: code,
    startupMs: { hello: marks.hello, ready: marks.ready, firstFrame: marks.firstFrame, bye: marks.bye },
    samples: marks.samples,
    frames: frames.length,
    bytes: { total: frames.reduce((a, f) => a + f.bytes, 0), p50: pct(frames.map((f) => f.bytes), 50), max: Math.max(0, ...frames.map((f) => f.bytes)) },
    stageMsP50: { pass: stage('pass'), paint: stage('paint'), readback: stage('readback'), diff: stage('diff'), encode: stage('encode') },
    drag: marks.dragFrom ? { frames: drag.length, after: inDrag.length - drag.length, fps: span ? (drag.length - 1) / span : null, latencyMs: { p50: pct(latency, 50), p95: pct(latency, 95) }, ...dragStages(drag) } : undefined,
    openMs: marks.openMs,
    rssMb: marks.rssEnd,
    click: measured.some((m) => m.group === 'click') ? { target: marks.click, ...group(measured.filter((m) => m.group === 'click').map(timed)) } : undefined,
    typing: measured.some((m) => m.group === 'typing') ? { target: marks.typing, ...group(measured.filter((m) => m.group === 'typing').map(timed)) } : undefined,
    idle: marks.idle,
    orbit: marks.orbit,
    edit: marks.edit,
    rename: marks.rename,
    export: marks.export,
    brand: marks.brand,
    idleFrames: marks.idleFrames,
    notes,
  }
}

const server = net.createServer((s) => {
  server.close()
  sock = s
  s.setNoDelay(true)
  s.on('data', reader(onMessage))
  s.on('error', (e) => notes.push(`leg A: ${e}`))
})
server.listen(0, '127.0.0.1', () => {
  const args = ['--stream', `127.0.0.1:${server.address().port}`, '--size', opt.size, '--dpr', opt.dpr, '--adapter', opt.adapter]
  if (opt.lang) args.push('--lang', opt.lang)
  child = spawn(opt.bin, args, {
    env: { ...process.env, SOLIDCRAFT_RENDER_TOKEN: renderToken, SOLIDCRAFT_CONTROL_TOKEN: controlToken },
    stdio: ['ignore', 'inherit', 'inherit'],
    windowsHide: true,
  })
  child.on('exit', (code) => {
    console.log(JSON.stringify(summary(code), null, 2))
    process.exit(code === 0 ? 0 : 1)
  })
})
setTimeout(() => {
  if (!marks.hello) notes.push('no hello in 15 s')
}, 15_000).unref()

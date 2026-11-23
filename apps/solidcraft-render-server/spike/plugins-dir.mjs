// Which plugins folder the render server's sessions use (Linux): a plugin planted in --config-dir DIR/plugins and a
// decoy in the desktop app's own folder (a fresh HOME's $XDG_DATA_HOME default, ~/.local/share/solidcraft/plugins);
// plugin.list over the control channel, run once with --config-dir and once without. Prints JSON; exit 1 when the
// folder does not follow --config-dir.
//   node plugins-dir.mjs --bin BIN --work DIR
import { spawn } from 'node:child_process'
import crypto from 'node:crypto'
import fs from 'node:fs'
import net from 'node:net'
import path from 'node:path'

const opt = {}
const a = process.argv.slice(2)
for (let i = 0; i < a.length; i += 2) opt[a[i].replace(/^--/, '')] = a[i + 1]
const work = path.resolve(opt.work)
const home = path.join(work, 'home'), config = path.join(work, 'config')
const plant = (dir, id) => {
  fs.mkdirSync(path.join(dir, id), { recursive: true })
  fs.writeFileSync(path.join(dir, id, 'plugin.json'), JSON.stringify({ id, name: id, version: '0.1.0', description: 'probe', capabilities: [] }))
}
plant(path.join(config, 'plugins'), 'sz-probe-config')
plant(path.join(home, '.local', 'share', 'solidcraft', 'plugins'), 'sz-probe-desktop')
const sleep = (ms) => new Promise((r) => setTimeout(r, ms))

async function listed(args) {
  const token = crypto.randomBytes(32).toString('hex')
  const env = { ...process.env, HOME: home, SOLIDCRAFT_CONTROL_TOKEN: token }
  delete env.XDG_DATA_HOME
  const out = path.join(work, `snap-${args.length ? 'config' : 'none'}`)
  const child = spawn(opt.bin, ['--snapshot', out, '--frames', '2', '--size', '640x400', '--hold-ms', '30000', '--adapter', 'software', ...args], { env, stdio: ['ignore', 'ignore', 'pipe'] })
  let log = ''
  child.stderr.on('data', (d) => (log += d))
  let port = null
  for (let i = 0; i < 600 && port === null; i++) {
    await sleep(100)
    const m = /^control 127\.0\.0\.1:(\d+)$/m.exec(log)
    if (m && /frames done/.test(log)) port = Number(m[1])
  }
  const reply = port === null ? { ok: false, error: 'no control port' } : await new Promise((resolve) => {
    const s = net.connect(port, '127.0.0.1', () => s.write(JSON.stringify({ id: 1, method: 'engine.execute', params: { command: 'plugin.list', params: {} }, token }) + '\n'))
    let buf = ''
    s.on('data', (d) => { buf += d; const nl = buf.indexOf('\n'); if (nl >= 0) { s.end(); resolve(JSON.parse(buf.slice(0, nl))) } })
    s.on('error', (e) => resolve({ ok: false, error: String(e) }))
  })
  child.kill()
  return { ok: reply.ok, error: reply.error ?? null, ids: (reply.result?.plugins ?? []).map((p) => p.manifest?.id ?? p.id) }
}

const withConfig = await listed(['--config-dir', config])
const without = await listed([])
const follows = withConfig.ids.includes('sz-probe-config') && !withConfig.ids.includes('sz-probe-desktop')
const control = without.ids.includes('sz-probe-desktop')
console.log(JSON.stringify({ withConfigDir: withConfig, withoutConfigDir: without, followsConfigDir: follows, probeSeesDesktopFolder: control }, null, 1))
process.exit(follows && control ? 0 : 1)

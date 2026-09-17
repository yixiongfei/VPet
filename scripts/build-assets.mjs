#!/usr/bin/env node
/**
 * build-assets.mjs —— 把 assets-src/pet/<pet>/ 下的原版 PNG 帧动画转成前端可用的资产。
 *
 *   assets-src/pet/vup/**              →  apps/desktop/public/pet/<clipId>/<n>.webp
 *   assets-src/pet/vup.lps             →  apps/desktop/public/pet/pet.json
 *                                          apps/desktop/public/pet/manifest.json
 *
 * 目录 → 动画信息的推断规则移植自 legacy/VPet-Simulator.Core/Handle/PetLoader.cs（LoadGraph）
 * 与 legacy/VPet-Simulator.Core/Graph/GraphInfo.cs（GraphInfo(path, info)），见 docs/05-body-assets.md §1。
 *
 * 用法：node scripts/build-assets.mjs [--pet vup] [--size 500] [--quality 85] [--force] [--dry]
 */
import fs from 'node:fs/promises'
import path from 'node:path'
import os from 'node:os'
import { fileURLToPath } from 'node:url'

const ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..')
const args = parseArgs(process.argv.slice(2))
const PET = args.pet ?? 'vup'
const SIZE = Number(args.size ?? 500)
const QUALITY = Number(args.quality ?? 85)
const FORCE = Boolean(args.force)
const DRY = Boolean(args.dry)

const SRC_ROOT = path.join(ROOT, 'assets-src', 'pet')
const PET_DIR = path.join(SRC_ROOT, PET)
const PET_LPS = path.join(SRC_ROOT, `${PET}.lps`)
const OUT_DIR = path.join(ROOT, 'apps', 'desktop', 'public', 'pet')
const FOOD_DIR = path.join(ROOT, 'assets-src', 'food')
/** 食物精灵在 500 的画布里最宽也就 ~65 逻辑像素，128 够 2 倍屏用了 */
const FOOD_SIZE = 128

/** 与 packages/shared/src/manifest.ts 的 GRAPH_TYPES 同序（原版 GraphType 枚举顺序） */
const GRAPH_TYPES = [
  'common', 'raised_dynamic', 'raised_static', 'move', 'default', 'touch_head', 'touch_body',
  'idel', 'sleep', 'say', 'stateone', 'statetwo', 'startup', 'shutdown', 'work',
  'switch_up', 'switch_down', 'switch_thirsty', 'switch_hunger',
  'sidehide_left_main', 'sidehide_left_rise', 'sidehide_right_main', 'sidehide_right_rise',
]
const GRAPH_TYPE_TOKENS = GRAPH_TYPES.map((t) => t.split('_'))
const MOODS = ['happy', 'nomal', 'poorcondition', 'ill']
const ANIMATS = { single: 'single', a_start: 'start', b_loop: 'loop', c_end: 'end' }
const GRAPH_LOADERS = new Set(['pnganimation', 'apnganimation', 'picture', 'foodanimation'])

main().catch((e) => {
  console.error(e)
  process.exit(1)
})

async function main() {
  const t0 = Date.now()
  await assertDir(PET_DIR, `找不到宠物资产目录 ${PET_DIR}`)

  // 1. 扫描：目录 → 原始 clip 列表
  const raw = []
  await loadGraphDir(PET_DIR, PET_DIR, raw)
  const frameCount = raw.reduce((n, c) => n + (c.files?.length ?? 0), 0)
  console.log(`扫描完成：${raw.filter((c) => !c._layered).length} 段动画，${frameCount} 帧，${raw.filter((c) => c._layered).length} 段夹心`)

  // 2. 分组成变体，生成 clip id
  const layeredRaw = raw.filter((c) => c._layered)
  const groups = new Map()
  for (const c of raw.filter((c) => !c._layered).sort((a, b) => a.source.localeCompare(b.source))) {
    const key = `${c.type}/${c.name}/${c.mood}/${c.animat}`
    if (!groups.has(key)) groups.set(key, [])
    groups.get(key).push(c)
  }
  const clips = []
  const index = {}
  for (const [key, list] of groups) {
    index[key] = []
    list.forEach((c, variant) => {
      const id = `${key}/${variant}`
      index[key].push(id)
      clips.push({
        id,
        type: c.type,
        name: c.name,
        mood: c.mood,
        animat: c.animat,
        variant,
        ...(c.layer ? { layer: c.layer } : {}),
        frames: c.files.map((f, i) => ({ src: `${id}/${String(i).padStart(3, '0')}.webp`, ms: f.ms })),
        totalMs: c.files.reduce((n, f) => n + f.ms, 0),
        source: path.relative(PET_DIR, c.dir).replaceAll('\\', '/'),
        _files: c.files,
      })
    })
  }

  // 3. 转码
  if (!DRY) {
    const sharp = (await import('sharp')).default
    sharp.concurrency(1) // 我们自己做并发，避免线程数爆炸
    await fs.mkdir(OUT_DIR, { recursive: true })
    const jobs = []
    for (const clip of clips) {
      const dir = path.join(OUT_DIR, clip.id)
      clip._files.forEach((f, i) => jobs.push({ clip, i, src: f.path, dst: path.join(dir, `${String(i).padStart(3, '0')}.webp`) }))
    }
    const failed = []
    let done = 0
    let skipped = 0
    const total = jobs.length
    const workers = Math.max(2, Math.min(os.cpus().length, 12))
    console.log(`转码 ${total} 帧 → ${SIZE}×${SIZE} WebP(q${QUALITY})，${workers} 并发…`)
    await Promise.all(
      Array.from({ length: workers }, async () => {
        for (;;) {
          const job = jobs.pop()
          if (!job) return
          const pngFallback = job.dst.replace(/\.webp$/, '.png')
          const usePng = () => { job.clip.frames[job.i].src = job.clip.frames[job.i].src.replace(/\.webp$/, '.png') }
          if (!FORCE && (await isFresh(job.src, job.dst))) {
            skipped++
          } else if (!FORCE && (await isFresh(job.src, pngFallback))) {
            skipped++
            usePng()
          } else {
            await fs.mkdir(path.dirname(job.dst), { recursive: true })
            try {
              await sharp(job.src, { failOn: 'none' })
                .resize(SIZE, SIZE, { fit: 'contain', background: { r: 0, g: 0, b: 0, alpha: 0 } })
                .webp({ quality: QUALITY, alphaQuality: 100, effort: 4 })
                .toFile(job.dst)
            } catch (e) {
              // 个别源文件 libspng 读不了：原样拷贝 PNG，manifest 里把这帧指向 .png（浏览器能解）
              failed.push(`${path.relative(PET_DIR, job.src)} — ${e.message}`)
              await fs.copyFile(job.src, pngFallback)
              usePng()
            }
          }
          done++
          if (done % 500 === 0) console.log(`  ${done}/${total}`)
        }
      }),
    )
    console.log(`转码完成：${done - skipped} 新生成，${skipped} 跳过（未变化）`)
    if (failed.length) {
      console.warn(`\n[warn] ${failed.length} 帧无法用 sharp 转码，已原样拷贝为 PNG：`)
      for (const f of failed) console.warn('  ' + f)
    }
  }

  // 3.5 夹心动画：把 info.lps 里写的层名解析成 clip id
  const layered = []
  for (const l of layeredRaw.sort((a, b) => a.source.localeCompare(b.source))) {
    const pick = (name) => index[`${l.type}/${name}/${l.mood}/${l.animat}`]?.[0]
    const back = pick(l.backName)
    const front = pick(l.frontName)
    if (!back || !front) {
      console.warn(`  [warn] 夹心动画 ${l.name}/${l.mood} 找不到层：back=${l.backName} front=${l.frontName}`)
      continue
    }
    layered.push({
      id: `${l.type}/${l.name}/${l.mood}/${l.animat}`,
      type: l.type, name: l.name, mood: l.mood, animat: l.animat,
      back, front, food: l.food,
      source: l.source.replaceAll('\\', '/'),
    })
  }

  // 3.6 食物：夹心动画中间那层的图
  const food = await buildFood()

  // 4. pet.json（vup.lps）
  const petJson = await fs
    .readFile(PET_LPS, 'utf8')
    .then((txt) => lpsToJson(parseLps(txt)))
    .catch(() => null)

  // 5. manifest.json
  const manifest = {
    pet: PET,
    size: SIZE,
    generatedAt: new Date().toISOString(),
    clips: clips.map(({ _files, ...c }) => c),
    index,
    layered,
    food,
  }
  if (!DRY) {
    await fs.writeFile(path.join(OUT_DIR, 'manifest.json'), JSON.stringify(manifest))
    if (petJson) await fs.writeFile(path.join(OUT_DIR, 'pet.json'), JSON.stringify(petJson, null, 2))
  }

  // 6. 统计
  const byType = {}
  for (const c of clips) byType[c.type] = (byType[c.type] ?? 0) + 1
  console.log('\n按类型统计：')
  for (const [t, n] of Object.entries(byType).sort((a, b) => b[1] - a[1])) console.log(`  ${t.padEnd(22)} ${n}`)
  console.log(`\nmanifest：${clips.length} clips · ${Object.keys(index).length} 键 · ${layered.length} 段夹心 · 用时 ${((Date.now() - t0) / 1000).toFixed(1)}s`)
  if (!DRY) console.log(`输出：${OUT_DIR}`)
}

/* ------------------------------------------------------------------ *
 * PetLoader.LoadGraph 的移植：递归目录，info.lps 优先，叶子目录自动生成
 * ------------------------------------------------------------------ */
async function loadGraphDir(dir, startup, out) {
  const entries = await fs.readdir(dir, { withFileTypes: true })
  const subdirs = entries.filter((e) => e.isDirectory())
  const infoPath = path.join(dir, 'info.lps')

  if (await exists(infoPath)) {
    const lines = parseLps(await fs.readFile(infoPath, 'utf8'))
    for (const line of lines) {
      if (!GRAPH_LOADERS.has(line.name.toLowerCase())) continue
      // 夹心动画：本身没有帧，只声明「后层 + 中间食物轨迹 + 前层」
      if (line.name.toLowerCase() === 'foodanimation') {
        out.push({
          ...graphInfo(dir, true, line, startup),
          _layered: true,
          dir,
          backName: (line.subs.back_lay ?? '').toLowerCase(),
          frontName: (line.subs.front_lay ?? '').toLowerCase(),
          food: parseFoodKeyframes(line.subs),
          source: path.relative(startup, dir),
        })
        continue
      }
      const rel = line.subs.path
      const p = rel ? path.join(dir, rel.replaceAll('\\', path.sep)) : dir
      const st = await fs.stat(p).catch(() => null)
      if (!st) {
        console.warn(`  [warn] info.lps 指向不存在的路径：${p}`)
        continue
      }
      if (st.isDirectory()) await addClipFromDir(p, startup, line, out)
      else await addClipFromFiles(path.dirname(p), [p], startup, line, out, /* isFile */ true)
    }
    return // 有 info.lps 的目录不再自动向下扫描（与原版一致）
  }

  if (subdirs.length === 0) {
    await addClipFromDir(dir, startup, emptyLine(), out)
    return
  }
  for (const d of subdirs) await loadGraphDir(path.join(dir, d.name), startup, out)
}

async function addClipFromDir(dir, startup, line, out) {
  const files = (await fs.readdir(dir))
    .filter((f) => /\.png$/i.test(f))
    .sort()
    .map((f) => path.join(dir, f))
  if (files.length === 0) return
  await addClipFromFiles(dir, files, startup, line, out, false)
}

async function addClipFromFiles(dir, files, startup, line, out, isFile) {
  const info = graphInfo(isFile ? files[0] : dir, !isFile, line, startup)
  out.push({
    ...info,
    dir,
    source: path.relative(startup, dir),
    files: files.map((p) => ({ path: p, ms: frameMs(p) })),
  })
}

/**
 * 食物：夹心动画中间那层。
 *
 * `assets-src/food/*.lps` 是原版的食物定义（名字 / 类型 / 用哪段动画 / 营养），
 * 图片按名字对应 `assets-src/food/image/<名字>.png`。
 * 只留 Body 和 Phase 2 的状态机用得到的字段——价格、经验、好感度是原版的养成经济，
 * 这个产品里没有。
 */
async function buildFood() {
  const lpsFiles = await fs.readdir(FOOD_DIR).catch(() => [])
  const items = []
  for (const f of lpsFiles.filter((f) => f.endsWith('.lps')).sort()) {
    for (const line of parseLps(await fs.readFile(path.join(FOOD_DIR, f), 'utf8'))) {
      if (line.name !== 'food') continue
      const name = line.subs.name
      const graph = (line.subs.graph ?? '').toLowerCase()
      if (!name || !graph) continue
      items.push({
        name,
        graph, // eat / drink / gift —— 决定用哪段夹心动画
        type: line.subs.type ?? '',
        strength: num(line.subs.Strength),
        strengthFood: num(line.subs.StrengthFood),
        strengthDrink: num(line.subs.StrengthDrink),
        feeling: num(line.subs.Feeling),
        health: num(line.subs.Health),
        _src: path.join(FOOD_DIR, 'image', `${name}.png`),
      })
    }
  }

  const out = []
  const outDir = path.join(OUT_DIR, 'food')
  if (!DRY) await fs.mkdir(outDir, { recursive: true })
  const sharp = DRY ? null : (await import('sharp')).default
  let missing = 0
  for (const [i, it] of items.entries()) {
    if (!(await exists(it._src))) { missing++; continue }
    const id = String(i).padStart(3, '0')
    const src = `food/${id}.webp`
    if (!DRY) {
      const dst = path.join(OUT_DIR, src)
      if (FORCE || !(await exists(dst))) {
        await sharp(it._src)
          .resize(FOOD_SIZE, FOOD_SIZE, { fit: 'inside', withoutEnlargement: true })
          .webp({ quality: QUALITY })
          .toFile(dst)
      }
    }
    const { _src, ...rest } = it
    out.push({ id, src, ...rest })
  }
  console.log(`食物：${out.length} 项${missing ? `（${missing} 项缺图，已跳过）` : ''}`)
  return out
}

const num = (v) => (v === undefined ? 0 : Number.parseFloat(v) || 0)

/**
 * 食物精灵的运动轨迹：`aN#时长,x,y,宽,旋转,不透明度`
 * （移植自 legacy FoodAnimation.Animation(ISub) 的构造）。
 * 只给一个值表示这段时间食物不显示，如 `a8#750`。
 */
function parseFoodKeyframes(subs) {
  const out = []
  for (let i = 0; subs[`a${i}`] !== undefined; i++) {
    const n = subs[`a${i}`].split(',').map((s) => Number.parseFloat(s))
    if (n.length === 1) out.push({ ms: n[0], visible: false })
    else {
      out.push({
        ms: n[0], visible: true, x: n[1], y: n[2], width: n[3],
        rotate: n.length > 4 ? n[4] : 0,
        opacity: n.length > 5 ? n[5] : 1,
      })
    }
  }
  return out
}

/** 帧时长：文件名最后一个 `_` 之后的数字（PNGAnimation.cs:227）；单图默认 1000ms */
function frameMs(file) {
  const base = path.basename(file, path.extname(file))
  const i = base.lastIndexOf('_')
  const n = i >= 0 ? Number.parseInt(base.slice(i + 1), 10) : NaN
  return Number.isFinite(n) && n > 0 ? n : 1000
}

/* ------------------------------------------------------------------ *
 * GraphInfo(path, info) 的移植
 * ------------------------------------------------------------------ */
function graphInfo(fsPath, isDir, line, startup) {
  const full = (isDir ? fsPath : fsPath.slice(0, fsPath.length - path.extname(fsPath).length)).toLowerCase()
  const rel = full.split(startup.toLowerCase()).pop() ?? ''
  const tokens = rel.replace(/[\\/]/g, '_').split('_').filter((t) => t.trim() !== '')

  // 1. 心情
  let mood = MOODS.find((m) => m === (line.subs.mode ?? '').toLowerCase())
  if (!mood) {
    mood = 'nomal'
    for (const m of MOODS) if (removeFirst(tokens, m)) { mood = m; break }
  }

  // 2. 类型：按枚举顺序找第一个 token 序列匹配
  let type = GRAPH_TYPES.find((t) => t === (line.subs.graph ?? '').toLowerCase())
  if (!type) {
    type = 'common'
    for (let i = 0; i < GRAPH_TYPE_TOKENS.length; i++) {
      const seq = GRAPH_TYPE_TOKENS[i]
      const at = tokens.indexOf(seq[0])
      if (at < 0) continue
      let ok = true
      for (let b = 1; b < seq.length && at + b < tokens.length; b++) if (tokens[at + b] !== seq[b]) { ok = false; break }
      if (ok) { type = GRAPH_TYPES[i]; tokens.splice(at, seq.length); break }
    }
  }

  // 3. 段落
  let animat = ANIMATS[(line.subs.animat ?? '').toLowerCase()]
  if (!animat) {
    if (removeFirst(tokens, 'a') || removeFirst(tokens, 'start')) animat = 'start'
    else if (removeFirst(tokens, 'b') || removeFirst(tokens, 'loop')) animat = 'loop'
    else if (removeFirst(tokens, 'c') || removeFirst(tokens, 'end')) animat = 'end'
    else { removeFirst(tokens, 'single'); animat = 'single' }
  }

  // 4. 名字
  let name = (line.info ?? '').trim().toLowerCase()
  if (!name) {
    while (tokens.length && (isNumeric(tokens.at(-1)) || tokens.at(-1).startsWith('~'))) tokens.pop()
    name = tokens.at(-1) ?? ''
  }
  if (!name) name = type

  // 带变体后缀的也算，如 eat_back_lay_2（info.lps 里就是这么命名的）
  const layer = /back_lay(_\d+)?$/.test(name) ? 'back' : /front_lay(_\d+)?$/.test(name) ? 'front' : undefined
  return { type, name, mood, animat, layer }
}

/* ------------------------------------------------------------------ *
 * LPS（LinePutScript）最小解析：每行 `name#info:|k#v:|k#v:|`，`///` 开头是注释
 * ------------------------------------------------------------------ */
function parseLps(text) {
  const lines = []
  for (const rawLine of text.split(/\r?\n/)) {
    const l = rawLine.trim()
    if (!l || l.startsWith('///')) continue
    const parts = l.split(':|').filter((p) => p !== '')
    if (parts.length === 0) continue
    const [name, info] = splitKV(parts[0])
    const subs = {}
    for (const p of parts.slice(1)) {
      const [k, v] = splitKV(p)
      subs[k] = v
    }
    lines.push({ name, info, subs })
  }
  return lines
}
function splitKV(s) {
  const i = s.indexOf('#')
  return i < 0 ? [s, ''] : [s.slice(0, i), s.slice(i + 1)]
}
function emptyLine() {
  return { name: 'pnganimation', info: '', subs: {} }
}
/** vup.lps → JSON：同名行合并为数组（work / move），其他为对象；数字自动转 number */
function lpsToJson(lines) {
  const out = {}
  for (const line of lines) {
    const obj = {}
    if (line.info) obj._ = coerce(line.info)
    for (const [k, v] of Object.entries(line.subs)) obj[k] = coerce(v)
    if (line.name in out) {
      if (!Array.isArray(out[line.name])) out[line.name] = [out[line.name]]
      out[line.name].push(obj)
    } else out[line.name] = obj
  }
  for (const k of ['work', 'move']) if (out[k] && !Array.isArray(out[k])) out[k] = [out[k]]
  return out
}
function coerce(v) {
  return /^-?\d+(\.\d+)?$/.test(v) ? Number(v) : v
}

/* ------------------------------------------------------------------ *
 * 小工具
 * ------------------------------------------------------------------ */
function removeFirst(arr, v) {
  const i = arr.indexOf(v)
  if (i < 0) return false
  arr.splice(i, 1)
  return true
}
function isNumeric(s) {
  return /^-?\d+(\.\d+)?$/.test(s)
}
async function exists(p) {
  return fs.access(p).then(() => true, () => false)
}
async function assertDir(p, msg) {
  const st = await fs.stat(p).catch(() => null)
  if (!st?.isDirectory()) throw new Error(msg)
}
async function isFresh(src, dst) {
  const [a, b] = await Promise.all([fs.stat(src).catch(() => null), fs.stat(dst).catch(() => null)])
  return Boolean(a && b && b.mtimeMs >= a.mtimeMs)
}
function parseArgs(argv) {
  const o = {}
  for (let i = 0; i < argv.length; i++) {
    const a = argv[i]
    if (!a.startsWith('--')) continue
    const k = a.slice(2)
    const next = argv[i + 1]
    if (next && !next.startsWith('--')) { o[k] = next; i++ } else o[k] = true
  }
  return o
}

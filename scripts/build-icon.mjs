// Crop the original character portrait; never redraw or replace the artist's design.
import sharp from 'sharp'
import fs from 'node:fs/promises'
import path from 'node:path'
import { fileURLToPath } from 'node:url'

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..')
const source = path.join(root, 'assets-src/pet/vup/Default/Happy/1/循环A_000_125.png')
const publicDir = path.join(root, 'apps/desktop/public')
await fs.mkdir(publicDir, { recursive: true })
// Coordinates in the source 1000px frame: face, hair and the signature hair clip.
const portrait = await sharp(source)
  .extract({ left: 326, top: 50, width: 370, height: 370 })
  .resize(512, 512)
  .png()
  .toBuffer()
await fs.writeFile(path.join(publicDir, 'avatar.png'), portrait)
await fs.writeFile(path.join(publicDir, 'icon.png'), portrait)
console.log('Portrait cropped to apps/desktop/public/avatar.png and icon.png')

// Real-cursor drag for live QA: posts mouse-down, a run of left-mouse-dragged
// steps and mouse-up at the HID level, which GPUI needs to start a drag.
//
// usage: osascript -l JavaScript script/drag.js X1 Y1 X2 Y2 [options]
//   coordinates are global screen points (top-left origin) unless --window is given
//   --window BUNDLE_ID   treat coordinates as relative to that app's front window
//   --scale S            divide coordinates by S first (2 for Retina screenshot pixels)
//   --steps N            dragged events between the points (default 24)
//   --step-ms MS         pause between dragged events (default 16)
//   --hold-ms MS         pause after mouse-down and before mouse-up (default 150)
//   --dry-run            print the resolved points and post nothing
//   --move-only          move the cursor to the start point and stop, to check aim by hover
//
// It moves the real cursor: nobody may touch the mouse while it runs.

ObjC.import('CoreGraphics')

const MOUSE_MOVED = 5, LEFT_DOWN = 1, LEFT_UP = 2, LEFT_DRAGGED = 6
const HID_TAP = 0, CLICK_STATE = 1

function run(argv) {
  const opts = { steps: 24, stepMs: 16, holdMs: 150, scale: 1, window: null, dryRun: false }
  const nums = []
  for (let i = 0; i < argv.length; i++) {
    const a = argv[i]
    if (a === '--window') opts.window = argv[++i]
    else if (a === '--scale') opts.scale = Number(argv[++i])
    else if (a === '--steps') opts.steps = Number(argv[++i])
    else if (a === '--step-ms') opts.stepMs = Number(argv[++i])
    else if (a === '--hold-ms') opts.holdMs = Number(argv[++i])
    else if (a === '--dry-run') opts.dryRun = true
    else if (a === '--move-only') opts.moveOnly = true
    else nums.push(Number(a))
  }
  if (nums.length !== 4 || nums.some(isNaN)) throw new Error('usage: drag.js X1 Y1 X2 Y2 [--window BUNDLE_ID] [--scale S] [--steps N] [--step-ms MS] [--hold-ms MS] [--dry-run]')

  let ox = 0, oy = 0
  if (opts.window) {
    const procs = Application('System Events').processes.whose({ bundleIdentifier: opts.window })
    if (procs.length === 0) throw new Error(`no running process with bundle id ${opts.window}`)
    if (procs[0].windows.length === 0) throw new Error(`${opts.window} has no open window`)
    const pos = procs[0].windows[0].position()
    ox = pos[0]; oy = pos[1]
  }
  const pt = (x, y) => ({ x: ox + x / opts.scale, y: oy + y / opts.scale })
  const from = pt(nums[0], nums[1]), to = pt(nums[2], nums[3])

  if (opts.dryRun) return JSON.stringify({ from, to, windowOrigin: { x: ox, y: oy }, ...opts })

  if (opts.window) {
    const procs = Application('System Events').processes.whose({ bundleIdentifier: opts.window })
    procs[0].frontmost = true
    delay(0.3)
  }

  const post = (type, p) => {
    const ev = $.CGEventCreateMouseEvent(null, type, p, 0)
    if (type !== MOUSE_MOVED) $.CGEventSetIntegerValueField(ev, CLICK_STATE, 1)
    $.CGEventPost(HID_TAP, ev)
  }

  post(MOUSE_MOVED, from)
  if (opts.moveOnly) return JSON.stringify({ moved: true, from })
  delay(opts.holdMs / 1000)
  post(LEFT_DOWN, from)
  delay(opts.holdMs / 1000)
  for (let i = 1; i <= opts.steps; i++) {
    const t = i / opts.steps
    post(LEFT_DRAGGED, { x: from.x + (to.x - from.x) * t, y: from.y + (to.y - from.y) * t })
    delay(opts.stepMs / 1000)
  }
  delay(opts.holdMs / 1000)
  post(LEFT_UP, to)
  return JSON.stringify({ posted: true, from, to })
}

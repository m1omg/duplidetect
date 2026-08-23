import AppKit

// Draws the DupliDetect icon: two offset waveform cards, the rear one faded,
// expressing "the same sound, stored twice".
func drawIcon(size: CGFloat) -> NSBitmapImageRep {
    let rep = NSBitmapImageRep(bitmapDataPlanes: nil, pixelsWide: Int(size), pixelsHigh: Int(size),
                              bitsPerSample: 8, samplesPerPixel: 4, hasAlpha: true, isPlanar: false,
                              colorSpaceName: .deviceRGB, bytesPerRow: 0, bitsPerPixel: 0)!
    NSGraphicsContext.saveGraphicsState()
    NSGraphicsContext.current = NSGraphicsContext(bitmapImageRep: rep)
    let ctx = NSGraphicsContext.current!.cgContext
    let s = size

    // Rounded-rect background with a blue-violet gradient.
    let bg = CGPath(roundedRect: CGRect(x: 0, y: 0, width: s, height: s),
                    cornerWidth: s * 0.2237, cornerHeight: s * 0.2237, transform: nil)
    ctx.saveGState()
    ctx.addPath(bg); ctx.clip()
    let colors = [NSColor(srgbRed: 0.30, green: 0.44, blue: 0.98, alpha: 1).cgColor,
                  NSColor(srgbRed: 0.52, green: 0.28, blue: 0.92, alpha: 1).cgColor] as CFArray
    let gradient = CGGradient(colorsSpace: CGColorSpaceCreateDeviceRGB(), colors: colors, locations: [0, 1])!
    ctx.drawLinearGradient(gradient, start: CGPoint(x: 0, y: s), end: CGPoint(x: s, y: 0), options: [])
    ctx.restoreGState()

    // Two stacked cards, the back one showing through.
    func card(offset: CGFloat, alpha: CGFloat) {
        let w = s * 0.52, h = s * 0.40
        let rect = CGRect(x: (s - w) / 2 + offset, y: (s - h) / 2 - offset, width: w, height: h)
        let path = CGPath(roundedRect: rect, cornerWidth: s * 0.05, cornerHeight: s * 0.05, transform: nil)
        ctx.saveGState()
        ctx.setFillColor(NSColor(white: 1, alpha: alpha).cgColor)
        ctx.addPath(path); ctx.fillPath()
        ctx.restoreGState()
        return
    }
    card(offset: s * 0.075, alpha: 0.35)
    card(offset: -s * 0.02, alpha: 1.0)

    // Waveform bars on the front card.
    let heights: [CGFloat] = [0.28, 0.55, 0.85, 0.45, 1.0, 0.62, 0.34, 0.72, 0.40]
    let w = s * 0.52
    let cx = (s - w) / 2 - s * 0.02
    let barW = s * 0.030
    let gap = (w - CGFloat(heights.count) * barW) / CGFloat(heights.count + 1)
    ctx.setFillColor(NSColor(srgbRed: 0.36, green: 0.35, blue: 0.92, alpha: 1).cgColor)
    for (i, hf) in heights.enumerated() {
        let bh = s * 0.22 * hf
        let x = cx + gap + CGFloat(i) * (barW + gap)
        let y = (s - s * 0.04) / 2 - bh / 2
        ctx.addPath(CGPath(roundedRect: CGRect(x: x, y: y, width: barW, height: bh),
                           cornerWidth: barW / 2, cornerHeight: barW / 2, transform: nil))
    }
    ctx.fillPath()

    NSGraphicsContext.restoreGraphicsState()
    return rep
}

let out = CommandLine.arguments[1]
for size in [16, 32, 64, 128, 256, 512, 1024] {
    let rep = drawIcon(size: CGFloat(size))
    let data = rep.representation(using: .png, properties: [:])!
    try! data.write(to: URL(fileURLWithPath: "\(out)/icon_\(size).png"))
}
print("icons written")

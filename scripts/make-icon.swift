// Draws the app icon: swift scripts/make-icon.swift <out.png> (1024×1024).
// Dark rounded square on Apple's icon grid (824 pt body, 100 pt margin) with the orange text cursor.
import AppKit

let out = CommandLine.arguments[1]
let size = 1024.0
let rep = NSBitmapImageRep(bitmapDataPlanes: nil, pixelsWide: 1024, pixelsHigh: 1024, bitsPerSample: 8,
                           samplesPerPixel: 4, hasAlpha: true, isPlanar: false, colorSpaceName: .deviceRGB,
                           bytesPerRow: 0, bitsPerPixel: 0)!
NSGraphicsContext.current = NSGraphicsContext(bitmapImageRep: rep)

let body = NSRect(x: 100, y: 100, width: 824, height: 824)
let shape = NSBezierPath(roundedRect: body, xRadius: 185, yRadius: 185)
NSGraphicsContext.saveGraphicsState()
let shadow = NSShadow()
shadow.shadowColor = .black.withAlphaComponent(0.35)
shadow.shadowOffset = NSSize(width: 0, height: -12)
shadow.shadowBlurRadius = 28
shadow.set()
NSColor.black.setFill()
shape.fill()
NSGraphicsContext.restoreGraphicsState()
NSGradient(starting: NSColor(white: 0.20, alpha: 1), ending: NSColor(white: 0.06, alpha: 1))!.draw(in: shape, angle: -90)
NSColor(white: 1, alpha: 0.12).setStroke()
shape.lineWidth = 4
shape.stroke()

let config = NSImage.SymbolConfiguration(pointSize: 440, weight: .semibold)
    .applying(.init(paletteColors: [.orange]))
let symbol = NSImage(systemSymbolName: "text.cursor", accessibilityDescription: nil)!.withSymbolConfiguration(config)!
let s = symbol.size
symbol.draw(in: NSRect(x: (size - s.width) / 2, y: (size - s.height) / 2, width: s.width, height: s.height))

NSGraphicsContext.current = nil
try! rep.representation(using: .png, properties: [:])!.write(to: URL(fileURLWithPath: out))

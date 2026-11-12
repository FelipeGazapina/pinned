// A throwaway app that asks for the microphone once and writes what macOS answered.
// scripts/mic-probe/run.sh builds it twice, with and without the audio-input entitlement.
import AVFoundation
import Foundation

let out = CommandLine.arguments.dropFirst().first ?? "/tmp/mic-probe.txt"
let before = AVCaptureDevice.authorizationStatus(for: .audio).rawValue
let started = Date()
let done = DispatchSemaphore(value: 0)
var granted = false
AVCaptureDevice.requestAccess(for: .audio) { ok in
    granted = ok
    done.signal()
}
let answered = done.wait(timeout: .now() + 20) == .success
let line = "statusBefore=\(before) answered=\(answered) granted=\(granted) seconds=\(String(format: "%.1f", Date().timeIntervalSince(started)))\n"
try? line.write(toFile: out, atomically: true, encoding: .utf8)

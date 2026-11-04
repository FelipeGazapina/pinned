// pinned dictation helper (macOS only): on-device speech recognition that streams
// interim + final results as JSON lines on stdout. Built by build.rs with swiftc.
//
// Protocol (stdout, one JSON object per line):
//   {"type":"ready"}                       — recognizer + mic are up
//   {"type":"result","text":"...","isFinal":false} — interim (provisional) text
//   {"type":"result","text":"...","isFinal":true}  — confirmed phrase
//   {"type":"error","message":"..."}       — fatal; the host should stop us
// Commands (stdin, one JSON object per line):
//   {"cmd":"stop"} — finish cleanly (exit 0)
//
// We restart the recognition task after every final so one session spans many
// phrases. The session ends only when the host says so — never on silence.

import AVFoundation
import Foundation
import Speech

func emit(_ obj: [String: Any]) {
    guard let data = try? JSONSerialization.data(withJSONObject: obj, options: []) else { return }
    FileHandle.standardOutput.write(data)
    FileHandle.standardOutput.write(Data([0x0A]))
}

func fail(_ message: String) -> Never {
    emit(["type": "error", "message": message])
    exit(1)
}

let localeId = CommandLine.arguments.dropFirst().first ?? "en-US"
let recognizer = SFSpeechRecognizer(locale: Locale(identifier: localeId))
guard let recognizer else {
    fail("No speech recognizer for \(localeId)")
}

let authSem = DispatchSemaphore(value: 0)
var authError: String?
SFSpeechRecognizer.requestAuthorization { status in
    if status != .authorized {
        authError = "Speech recognition not authorized (\(status.rawValue))"
    }
    authSem.signal()
}
authSem.wait()
if let authError { fail(authError) }

let micSem = DispatchSemaphore(value: 0)
var micGranted = false
AVCaptureDevice.requestAccess(for: .audio) { granted in
    micGranted = granted
    micSem.signal()
}
micSem.wait()
if !micGranted {
    fail("Microphone access denied")
}

let engine = AVAudioEngine()
let input = engine.inputNode
let format = input.outputFormat(forBus: 0)

var stopping = false
var restarts = 0
var currentRequest: SFSpeechAudioBufferRecognitionRequest?
var lastText = ""

func startTask() {
    if stopping { return }
    let request = SFSpeechAudioBufferRecognitionRequest()
    request.shouldReportPartialResults = true
    // Prefer the downloaded on-device model: free, offline, nothing leaves the Mac.
    if #available(macOS 13, *) {
        request.requiresOnDeviceRecognition = true
    }
    currentRequest = request
    lastText = ""
    recognizer.recognitionTask(with: request) { result, error in
        if stopping { return }
        if let result {
            let text = result.bestTranscription.formattedString
            if text != lastText, !text.isEmpty {
                lastText = text
                emit(["type": "result", "text": text, "isFinal": result.isFinal])
            }
            if result.isFinal {
                // One phrase confirmed; open a fresh task for the next one.
                DispatchQueue.main.async { startTask() }
                return
            }
        }
        if let error {
            let ns = error as NSError
            // 1110 (kAFAssistantErrorDomain) is our own cancel; anything else restarts a few times.
            if ns.code == 1110 { return }
            restarts += 1
            if restarts > 5 {
                emit(["type": "error", "message": error.localizedDescription])
                exit(1)
            }
            DispatchQueue.main.asyncAfter(deadline: .now() + 0.5) { startTask() }
        }
    }
}

// The tap outlives every task: it always feeds the current request.
input.installTap(onBus: 0, bufferSize: 1024, format: format) { buffer, _ in
    currentRequest?.append(buffer)
}

// Stop command on stdin.
Thread.detachNewThread {
    while let line = readLine(strippingNewline: true) {
        if line.contains("\"stop\"") { break }
    }
    stopping = true
    currentRequest?.endAudio()
    // Let the final flush through, then go quiet.
    Thread.sleep(forTimeInterval: 1.0)
    engine.stop()
    input.removeTap(onBus: 0)
    exit(0)
}

do {
    try engine.start()
} catch {
    fail("Could not start the microphone: \(error.localizedDescription)")
}
startTask()
emit(["type": "ready"])

// Block until killed or stdin closes (the host stopped us).
RunLoop.main.run()

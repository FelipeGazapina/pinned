// pinned dictation helper (macOS only): on-device speech recognition that streams
// interim + final results as JSON lines on stdout. Built by build.rs with swiftc.
//
// Usage: dictation-helper <locale>, for example en-US or pt-BR.
//
// Protocol (stdout, one JSON object per line):
//   {"type":"ready"}                       — recognizer + mic are up
//   {"type":"notice","message":"..."}      — progress worth showing, e.g. a model download
//   {"type":"result","text":"...","isFinal":false} — interim (provisional) text
//   {"type":"result","text":"...","isFinal":true}  — confirmed phrase
//   {"type":"error","message":"..."}       — fatal; the host should stop us
// Commands (stdin, one JSON object per line):
//   {"vocabulary":["commit","PR",...]} — first line only: words the recognizer should expect
//   {"cmd":"stop"} — finish cleanly (exit 0)
//
// Engines: on macOS 26+ SpeechAnalyzer + SpeechTranscriber, which runs Portuguese
// on device where SFSpeechRecognizer's on-device mode does not; older systems keep
// SFSpeechRecognizer. Both stay on device. One session spans many phrases and
// ends only when the host says so, never on silence.

import AVFoundation
import Foundation
import Speech

let output = NSLock()

func emit(_ obj: [String: Any]) {
    guard var data = try? JSONSerialization.data(withJSONObject: obj, options: []) else { return }
    data.append(0x0A)
    output.lock()
    FileHandle.standardOutput.write(data)
    output.unlock()
}

func fail(_ message: String) -> Never {
    emit(["type": "error", "message": message])
    exit(1)
}

let localeId = CommandLine.arguments.dropFirst().first ?? "en-US"
let vocabulary: [String] = {
    guard let line = readLine(strippingNewline: true) else { return [] }
    let config = (try? JSONSerialization.jsonObject(with: Data(line.utf8))) as? [String: Any]
    if config?["cmd"] as? String == "stop" { exit(0) }
    return config?["vocabulary"] as? [String] ?? []
}()
let english = Locale(identifier: "en")
let languageName = english.localizedString(forLanguageCode: localeId) ?? localeId
let localeName = english.localizedString(forIdentifier: localeId) ?? localeId

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
var shutdown: () -> Void = {
    engine.stop()
    exit(0)
}

func startMic() {
    do {
        try engine.start()
    } catch {
        fail("Could not start the microphone: \(error.localizedDescription)")
    }
}

@available(macOS 26, *)
func runAnalyzer() async {
    guard let locale = await SpeechTranscriber.supportedLocale(equivalentTo: Locale(identifier: localeId)) else {
        fail("\(localeName) dictation isn't available on this Mac")
    }
    // Live audio needs the progressive preset (volatile + fast results); with
    // volatile results alone the analyzer holds every result until input ends.
    let transcriber = SpeechTranscriber(locale: locale, preset: .progressiveTranscription)
    if await AssetInventory.status(forModules: [transcriber]) == .unsupported {
        fail("\(localeName) dictation isn't available on this Mac")
    }
    // A model on disk still reports .supported until this process claims it, and
    // the claim is quick; only a model missing from disk earns the notice.
    let onDisk = await SpeechTranscriber.installedLocales.contains { $0.identifier(.bcp47) == locale.identifier(.bcp47) }
    if !onDisk {
        emit(["type": "notice", "message": "Downloading the \(languageName) speech model…"])
    }
    do {
        try await AssetInventory.assetInstallationRequest(supporting: [transcriber])?.downloadAndInstall()
    } catch {
        fail("Could not download the \(languageName) speech model: \(error.localizedDescription)")
    }
    if !onDisk {
        emit(["type": "notice", "message": "\(languageName) speech model ready."])
    }
    guard let analyzerFormat = await SpeechAnalyzer.bestAvailableAudioFormat(compatibleWith: [transcriber], considering: format),
          let converter = AVAudioConverter(from: format, to: analyzerFormat)
    else {
        fail("No audio format the \(languageName) speech model accepts")
    }
    let analyzer = SpeechAnalyzer(modules: [transcriber])
    let (audio, feed) = AsyncStream<AnalyzerInput>.makeStream()

    Task {
        var lastText = ""
        do {
            for try await result in transcriber.results {
                if stopping { return }
                let text = String(result.text.characters).trimmingCharacters(in: .whitespacesAndNewlines)
                if text.isEmpty || (!result.isFinal && text == lastText) { continue }
                lastText = result.isFinal ? "" : text
                emit(["type": "result", "text": text, "isFinal": result.isFinal])
            }
        } catch {
            if !stopping { fail(error.localizedDescription) }
        }
    }

    let ratio = analyzerFormat.sampleRate / format.sampleRate
    input.installTap(onBus: 0, bufferSize: 4096, format: format) { buffer, _ in
        let capacity = AVAudioFrameCount((Double(buffer.frameLength) * ratio).rounded(.up)) + 16
        guard let converted = AVAudioPCMBuffer(pcmFormat: analyzerFormat, frameCapacity: capacity) else { return }
        var consumed = false
        var error: NSError?
        let status = converter.convert(to: converted, error: &error) { _, inputStatus in
            if consumed {
                inputStatus.pointee = .noDataNow
                return nil
            }
            consumed = true
            inputStatus.pointee = .haveData
            return buffer
        }
        if status != .error, converted.frameLength > 0 {
            feed.yield(AnalyzerInput(buffer: converted))
        }
    }

    do {
        if !vocabulary.isEmpty {
            let context = AnalysisContext()
            context.contextualStrings[.general] = vocabulary
            try await analyzer.setContext(context)
        }
        try await analyzer.prepareToAnalyze(in: analyzerFormat)
        try await analyzer.start(inputSequence: audio)
    } catch {
        fail("Could not start \(languageName) dictation: \(error.localizedDescription)")
    }
    startMic()
    emit(["type": "ready"])
}

func runRecognizer() {
    guard let recognizer = SFSpeechRecognizer(locale: Locale(identifier: localeId)) else {
        fail("No speech recognizer for \(localeId)")
    }
    var restarts = 0
    var currentRequest: SFSpeechAudioBufferRecognitionRequest?
    var lastText = ""

    func startTask() {
        if stopping { return }
        let request = SFSpeechAudioBufferRecognitionRequest()
        request.shouldReportPartialResults = true
        request.contextualStrings = vocabulary
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
    shutdown = {
        currentRequest?.endAudio()
        // Let the final flush through, then go quiet.
        Thread.sleep(forTimeInterval: 1.0)
        engine.stop()
        input.removeTap(onBus: 0)
        exit(0)
    }
    startMic()
    startTask()
    emit(["type": "ready"])
}

// Stop command on stdin.
Thread.detachNewThread {
    while let line = readLine(strippingNewline: true) {
        if line.contains("\"stop\"") { break }
    }
    stopping = true
    shutdown()
}

if #available(macOS 26, *), SpeechTranscriber.isAvailable {
    Task { await runAnalyzer() }
} else {
    runRecognizer()
}

// Block until killed or stdin closes (the host stopped us).
RunLoop.main.run()

import Combine
import CryptoKit
import Foundation

/// Resumable, checksum-verified download of one pinned file (spec §16). Data goes to
/// `<dest>.part`; an interrupted download continues from there with a Range request, and the
/// file only appears at `dest` once its sha256 matches.
final class Download: NSObject, ObservableObject, URLSessionDataDelegate {
    enum State: Equatable {
        case idle, running(Double), verifying, failed(String), done
    }

    @Published private(set) var state: State = .idle
    /// Called on the main queue once the verified file is in place.
    var onDone: (() -> Void)?

    private let url: URL
    private let dest: URL
    private let sha256: String
    private let size: Int64
    private var part: URL { dest.appendingPathExtension("part") }

    // Touched only on the session's serial delegate queue (and in `start`, before the task runs).
    private var handle: FileHandle?
    private var received: Int64 = 0
    private var lastShown = -1.0
    private var task: URLSessionDataTask?
    private var attempt = 0
    private let maxRetries = 3

    init(url: URL, dest: URL, sha256: String, size: Int64) {
        self.url = url
        self.dest = dest
        self.sha256 = sha256
        self.size = size
    }

    var isBusy: Bool {
        switch state {
        case .running, .verifying: true
        default: false
        }
    }

    func start() {
        guard !isBusy else { return }
        attempt = 0
        run()
    }

    /// Stops the transfer; the partial file stays so the next start resumes.
    func cancel() {
        guard case .running = state else { return }
        task?.cancel()
        set(.idle)  // also covers a pending retry
    }

    private func run() {
        let fm = FileManager.default
        do {
            try fm.createDirectory(at: dest.deletingLastPathComponent(), withIntermediateDirectories: true)
            if !fm.fileExists(atPath: part.path) { fm.createFile(atPath: part.path, contents: nil) }
            var have = (try fm.attributesOfItem(atPath: part.path)[.size] as? Int64) ?? 0
            if have > size {
                try fm.removeItem(at: part)
                fm.createFile(atPath: part.path, contents: nil)
                have = 0
            }
            let handle = try FileHandle(forWritingTo: part)
            try handle.seekToEnd()
            self.handle = handle
            received = have
            let free = try dest.deletingLastPathComponent()
                .resourceValues(forKeys: [.volumeAvailableCapacityForImportantUsageKey])
                .volumeAvailableCapacityForImportantUsage ?? .max
            if free < size - have {
                try handle.close()
                self.handle = nil
                return set(.failed("Not enough disk space: the model needs \(ByteCountFormatter.string(fromByteCount: size - have, countStyle: .file)) free."))
            }
        } catch {
            return set(.failed(error.localizedDescription))
        }
        set(.running(Double(received) / Double(size)))
        if received == size { return verify() }
        var request = URLRequest(url: url)
        if received > 0 { request.setValue("bytes=\(received)-", forHTTPHeaderField: "Range") }
        let t = URLSession(configuration: .default, delegate: self, delegateQueue: nil).dataTask(with: request)
        task = t
        t.resume()
    }

    private func set(_ s: State) {
        DispatchQueue.main.async { self.state = s }
    }

    // MARK: URLSessionDataDelegate (serial queue)

    func urlSession(_ session: URLSession, dataTask: URLSessionDataTask, didReceive response: URLResponse,
                    completionHandler: @escaping (URLSession.ResponseDisposition) -> Void) {
        let code = (response as? HTTPURLResponse)?.statusCode ?? 0
        if code == 200 && received > 0 {
            // The server ignored our Range request: start the file over.
            try? handle?.truncate(atOffset: 0)
            received = 0
        } else if code != 200 && code != 206 {
            if code == 416 { try? FileManager.default.removeItem(at: part) }  // stale partial: retry from zero
            set(.failed("Download failed (HTTP \(code))"))
            handle = nil
            return completionHandler(.cancel)
        }
        completionHandler(.allow)
    }

    func urlSession(_ session: URLSession, dataTask: URLSessionDataTask, didReceive data: Data) {
        do {
            try handle?.write(contentsOf: data)
        } catch {
            handle = nil
            dataTask.cancel()
            return set(.failed("Can't write the model: \(error.localizedDescription)"))
        }
        received += Int64(data.count)
        let fraction = Double(received) / Double(size)
        if fraction - lastShown >= 0.005 {
            lastShown = fraction
            set(.running(min(fraction, 1)))
        }
    }

    func urlSession(_ session: URLSession, task: URLSessionTask, didCompleteWithError error: Error?) {
        session.finishTasksAndInvalidate()
        guard handle != nil else { return }  // already failed above
        try? handle?.close()
        handle = nil
        if let error {
            if (error as NSError).code == NSURLErrorCancelled { return set(.idle) }
            // Network trouble: resume from the partial file after 2, 4, 8 s.
            guard attempt < maxRetries else { return set(.failed(error.localizedDescription)) }
            attempt += 1
            let delay = Double(1 << attempt)
            return DispatchQueue.main.asyncAfter(deadline: .now() + delay) { [weak self] in
                guard let self, case .running = self.state else { return }
                self.run()
            }
        }
        verify()
    }

    private func verify() {
        try? handle?.close()
        handle = nil
        set(.verifying)
        DispatchQueue.global(qos: .utility).async {
            do {
                let reader = try FileHandle(forReadingFrom: self.part)
                defer { try? reader.close() }
                var hasher = SHA256()
                while let chunk = try reader.read(upToCount: 4 << 20), !chunk.isEmpty { hasher.update(data: chunk) }
                let digest = hasher.finalize().map { String(format: "%02x", $0) }.joined()
                guard digest == self.sha256 else {
                    try? FileManager.default.removeItem(at: self.part)
                    return self.set(.failed("Checksum mismatch: the download was corrupted. Try again."))
                }
                _ = try FileManager.default.replaceItemAt(self.dest, withItemAt: self.part)
                DispatchQueue.main.async {
                    self.state = .done
                    self.onDone?()
                }
            } catch {
                self.set(.failed(error.localizedDescription))
            }
        }
    }
}

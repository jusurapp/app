import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";

interface ProgressPayload {
  status: string;
  message: string;
  progress: number;
  speed?: number;
}

interface VideoMetadata {
  video_id: string;
  url: string;
  title: string;
  thumbnail_url?: string;
  author_name?: string;
  duration_secs?: number;
  site: string;
  created_at: number;
  segment_count?: number;
}

interface TranslationStatus {
  video_id: string;
  message: string;
}

function formatSpeed(bps: number): string {
  if (bps >= 1024 * 1024) return `${(bps / (1024 * 1024)).toFixed(1)} MB/s`;
  if (bps >= 1024) return `${(bps / 1024).toFixed(0)} KB/s`;
  return `${bps.toFixed(0)} B/s`;
}

function formatDuration(secs: number): string {
  const h = Math.floor(secs / 3600);
  const m = Math.floor((secs % 3600) / 60);
  const s = secs % 60;
  if (h > 0)
    return `${h}:${String(m).padStart(2, "0")}:${String(s).padStart(2, "0")}`;
  return `${m}:${String(s).padStart(2, "0")}`;
}

function formatRelativeTime(unixSecs: number): string {
  const diff = Math.floor(Date.now() / 1000) - unixSecs;
  if (diff < 60) return "just now";
  if (diff < 3600) return `${Math.floor(diff / 60)}m ago`;
  if (diff < 86400) return `${Math.floor(diff / 3600)}h ago`;
  return `${Math.floor(diff / 86400)}d ago`;
}

const CHROME_EXTENSION_URL = "https://chromewebstore.google.com/detail/TODO";

function Logo() {
  return (
    <div className="flex items-center gap-2" style={{ color: "#856451" }}>
      <svg
        xmlns="http://www.w3.org/2000/svg"
        viewBox="127 265.09 820 420.91"
        fill="currentColor"
        style={{ height: "1.5rem", width: "auto" }}
      >
        <path d="M216.1 676.4c-3.4-25.9-11.5-44.2-26.5-60.1-16.1-17.1-35.3-27-55.8-29l-6.8-.6V512h95v-32.4c0-36.8 1.1-48.9 6.5-70.2 13.6-54.4 47.7-98.4 95.8-123.9 44.2-23.5 101.8-27 148.7-9 26.9 10.3 46.7 23 66 42.4 26.6 26.8 42.8 55.3 50.9 89.9 5 21.4 5 21.3 5.1 153.2l.2 123.5-48.4.3-48.4.2-1.2-10.9c-4.9-43.8-31.7-75.9-71.8-85.8-20.8-5.1-47.9.6-66.7 14.1-22.3 15.8-35.4 40.1-38.8 71.7l-1.2 10.9H217.4zM497 497.5c0-14 .1-14.8 2.9-20.3 4-7.9 7.6-18.5 9.1-26.3 3-16.7-5.4-39.4-20.9-56.1-9.8-10.7-22.8-19.5-53.1-35.8-7.4-4-16.3-9.2-19.7-11.6l-6.3-4.3-6.2 4.3c-3.5 2.4-12.4 7.6-19.8 11.7-21.6 11.6-31.3 17.5-39.7 23.8-22.4 17-34.5 37.8-34.6 59.6 0 11.2 2 18.7 8.4 32.6l4.9 10.6V512h175z" />
        <path d="M759.6 678.8c-1.4-15.5-7.1-34-13.7-45-7.5-12.2-18.8-24.5-29.4-31.8-10.8-7.4-26.4-13.1-40-14.6-12.2-1.3-31.3 3.7-44.7 11.6-3.9 2.4-7.9 4-9.7 4H619l.2-45.2.3-45.3 68.3-.3 68.2-.2v-28.1l4.4-9.2c12.2-25.3 11-45.5-4-68.3-10-15.1-26.3-28-53.9-42.7-14.9-7.9-25.9-14.4-31.2-18.3l-3.2-2.4-6.5 4.5c-3.5 2.5-13.8 8.4-22.8 13.1-9 4.8-19.7 10.8-23.9 13.5-5.1 3.3-8.7 4.9-10.8 4.9-2.9 0-3.4-.5-4.9-4.2-7.9-20.4-22.4-43.3-38.2-60.2-5.5-5.9-6.1-6.9-5.8-10.2s1.1-4.4 7.2-9c8-6.1 26.7-16 37.6-19.9 28.1-10 65.9-13 99.4-8 67.4 10.2 122.5 58.3 145 126.5 8.7 26.4 10.6 41.9 10.6 87.2V512l45.8.2 45.7.3.3 37 .2 37.1-9.2 1.2c-30.6 4.2-56.4 23.3-70.5 52.3-4.8 9.7-10 28.1-10.9 38.7l-.7 7.2h-95.4z" />
      </svg>
      <span
        style={{ fontFamily: "'DM Serif Display', serif", fontSize: "1.5rem" }}
      >
        Jusur
      </span>
    </div>
  );
}

function IslamicStar({ style }: { style: React.CSSProperties }) {
  return (
    <svg
      style={style}
      viewBox="0 0 100 100"
      xmlns="http://www.w3.org/2000/svg"
      className="absolute pointer-events-none"
    >
      <g fill="none" stroke="#856451" strokeWidth="1.5">
        <rect x="18" y="18" width="64" height="64" />
        <rect
          x="18"
          y="18"
          width="64"
          height="64"
          transform="rotate(45 50 50)"
        />
        <circle cx="50" cy="50" r="16" />
      </g>
    </svg>
  );
}

function Thumbnail({ url, className }: { url?: string; className?: string }) {
  if (url) {
    return (
      <img
        src={url}
        className={`w-full h-full object-cover ${className ?? ""}`}
      />
    );
  }
  return (
    <div
      className={`w-full h-full flex items-center justify-center bg-white/5 ${className ?? ""}`}
    >
      <svg
        className="w-5 h-5 text-white/20"
        fill="currentColor"
        viewBox="0 0 24 24"
      >
        <path d="M8 5v14l11-7z" />
      </svg>
    </div>
  );
}

function App() {
  const [step, setStep] = useState<1 | 2 | 3>(1);
  const [statusMessage, setStatusMessage] = useState("");
  const [progress, setProgress] = useState(0);
  const [downloadSpeed, setDownloadSpeed] = useState<number | null>(null);
  const [activeTranslations, setActiveTranslations] = useState<
    Record<string, { metadata: VideoMetadata; status: string }>
  >({});
  const [history, setHistory] = useState<VideoMetadata[]>([]);

  useEffect(() => {
    // Skip setup if everything is already installed
    Promise.all([
      invoke<boolean>("check_llama_model"),
      invoke<boolean>("check_ytdlp_installed"),
      invoke<boolean>("check_whisper_model"),
    ]).then(([llamaModel, ytdlp, whisperModel]) => {
      if (llamaModel && ytdlp && whisperModel) {
        setStep(3);
      }
    });
  }, []);

  // Step 2 orchestration
  useEffect(() => {
    if (step !== 2) return;

    let cancelled = false;

    const handleProgress = (event: { payload: ProgressPayload }) => {
      if (cancelled) return;
      setStatusMessage(event.payload.message);
      setProgress(event.payload.progress);
      setDownloadSpeed(event.payload.speed ?? null);
    };

    const unlistenLlamaModel = listen<ProgressPayload>(
      "llama-model-progress",
      handleProgress,
    );
    const unlistenWhisperModel = listen<ProgressPayload>(
      "whisper-model-progress",
      handleProgress,
    );
    const unlistenYtdlp = listen<ProgressPayload>(
      "ytdlp-progress",
      handleProgress,
    );

    const run = async () => {
      try {
        // Check & download LLM model
        setStatusMessage("Checking LLM model...");
        const modelCached = await invoke<boolean>("check_llama_model");
        if (!modelCached) {
          await invoke("download_llama_model");
          if (cancelled) return;
        }

        // Check & install yt-dlp
        setStatusMessage("Checking yt-dlp...");
        const ytdlpInstalled = await invoke<boolean>("check_ytdlp_installed");
        if (!ytdlpInstalled) {
          setStatusMessage("Installing yt-dlp...");
          await invoke("install_ytdlp");
          if (cancelled) return;
        }

        // Check & download whisper model
        setStatusMessage("Checking whisper model...");
        const whisperModelReady = await invoke<boolean>("check_whisper_model");
        if (!whisperModelReady) {
          await invoke("download_whisper_model");
          if (cancelled) return;
        }

        setStatusMessage("Ready to translate.");
        setProgress(100);
        setTimeout(() => setStep(3), 1500);
      } catch (e) {
        if (cancelled) return;
        setStatusMessage(`Error: ${e}`);
      }
    };

    run();

    return () => {
      cancelled = true;
      unlistenLlamaModel.then((f) => f());
      unlistenWhisperModel.then((f) => f());
      unlistenYtdlp.then((f) => f());
    };
  }, [step]);

  // Step 3: dashboard listeners
  useEffect(() => {
    if (step !== 3) return;

    invoke<VideoMetadata[]>("get_history").then(setHistory);

    // Auto-update yt-dlp to latest version in background
    invoke("update_ytdlp").catch(() => {});

    const unlistenStarted = listen<VideoMetadata>(
      "translation-started",
      (e) => {
        const id = e.payload.video_id;
        setActiveTranslations((prev) => ({
          ...prev,
          [id]: { metadata: e.payload, status: "Starting..." },
        }));
      },
    );

    const unlistenStatus = listen<TranslationStatus>(
      "translation-status",
      (e) => {
        const id = e.payload.video_id;
        setActiveTranslations((prev) =>
          prev[id]
            ? { ...prev, [id]: { ...prev[id], status: e.payload.message } }
            : prev,
        );
      },
    );

    const unlistenCompleted = listen<VideoMetadata>(
      "translation-completed",
      (e) => {
        const id = e.payload.video_id;
        setActiveTranslations((prev) => {
          const next = { ...prev };
          delete next[id];
          return next;
        });
        setHistory((prev) => [
          e.payload,
          ...prev.filter((h) => h.video_id !== id),
        ]);
      },
    );

    return () => {
      unlistenStarted.then((f) => f());
      unlistenStatus.then((f) => f());
      unlistenCompleted.then((f) => f());
    };
  }, [step]);

  const handleDelete = async (videoId: string) => {
    setHistory((prev) => prev.filter((h) => h.video_id !== videoId));
    await invoke("delete_translation", { videoId });
  };

  const handleRedo = async (item: VideoMetadata) => {
    setHistory((prev) => prev.filter((h) => h.video_id !== item.video_id));
    invoke("redo_translation", { videoId: item.video_id });
  };

  return (
    <div className="flex flex-col h-screen bg-[#232527] text-white overflow-hidden select-none font-dm-sans">
      {/* Background decorations — only on setup steps */}
      {step !== 3 && (
        <div className="absolute inset-0 pointer-events-none overflow-hidden opacity-[0.25]">
          <IslamicStar
            style={{
              top: "8%",
              left: "5%",
              width: 110,
              transform: "rotate(12deg)",
            }}
          />
          <IslamicStar
            style={{
              bottom: "12%",
              right: "6%",
              width: 85,
              transform: "rotate(-8deg)",
            }}
          />
          <IslamicStar
            style={{
              top: "35%",
              right: "14%",
              width: 55,
              transform: "rotate(22deg)",
            }}
          />
        </div>
      )}

      {step === 1 && (
        <div className="flex flex-col items-center justify-center flex-1 gap-7">
          <Logo />

          <h1
            className="text-[2.6rem] font-semibold tracking-tight leading-none"
            style={{ fontFamily: "'Playfair Display', serif" }}
          >
            Welcome.
          </h1>

          <p className="text-white/40 text-sm text-center whitespace-nowrap">
            Install the Jusur Chrome extension to get started.
          </p>

          <button
            onClick={() => invoke("open_url", { url: CHROME_EXTENSION_URL })}
            className="rounded-xl bg-[#856451] text-white/90 hover:bg-[#9a7661] transition-all text-sm font-medium cursor-pointer"
            style={{ padding: "8px 20px" }}
          >
            Get Chrome Extension
          </button>

          <button
            onClick={() => setStep(2)}
            className="flex items-center gap-1.5 text-white/40 hover:text-white/60 transition-colors text-sm cursor-pointer"
          >
            Next
            <svg
              className="w-3.5 h-3.5"
              fill="none"
              viewBox="0 0 24 24"
              stroke="currentColor"
              strokeWidth={2}
            >
              <path
                strokeLinecap="round"
                strokeLinejoin="round"
                d="M9 5l7 7-7 7"
              />
            </svg>
          </button>
        </div>
      )}

      {step === 2 && (
        <div className="flex flex-col items-center justify-center flex-1 gap-10">
          <Logo />

          {progress < 100 ? (
            <>
              <h1
                className="text-[2rem] font-semibold tracking-tight"
                style={{ fontFamily: "'Playfair Display', serif" }}
              >
                Setting things up.
              </h1>

              <div className="w-72 flex flex-col items-center gap-3">
                <p className="text-sm text-white/40 text-center">
                  {statusMessage}
                </p>
                <div className="w-full bg-white/[0.07] rounded-full h-[3px] overflow-hidden">
                  <div
                    className="h-full rounded-full transition-all duration-500"
                    style={{
                      width: `${Math.max(progress, 2)}%`,
                      backgroundColor: "#856451",
                    }}
                  />
                </div>
                {downloadSpeed !== null && downloadSpeed > 0 && (
                  <p className="text-xs text-white/25">
                    {formatSpeed(downloadSpeed)}
                  </p>
                )}
              </div>
            </>
          ) : null}
        </div>
      )}

      {step === 3 && (
        <div className="flex flex-col flex-1 min-h-0">
          {/* Header */}
          <div
            className="flex flex-col items-center justify-center px-5 border-b border-white/[0.05] flex-shrink-0"
            style={{ paddingTop: "10px", paddingBottom: "10px" }}
          >
            <Logo />
          </div>

          {/* Scrollable content */}
          <div className="flex-1 overflow-y-auto px-5 py-4 flex flex-col gap-2.5">
            {/* Active translations */}
            {Object.values(activeTranslations).map((active) => (
              <div
                key={active.metadata.video_id}
                className="border border-[#856451]/40 bg-[#856451]/[0.06] p-3.5 flex gap-3.5 items-center"
              >
                <div className="w-28 h-16 overflow-hidden flex-shrink-0 bg-black/40">
                  <Thumbnail url={active.metadata.thumbnail_url} />
                </div>
                <div className="flex flex-col gap-1.5 min-w-0">
                  <p className="text-sm font-medium text-white/90 leading-snug line-clamp-2">
                    {active.metadata.title}
                  </p>
                  <p className="text-xs text-[#856451]">{active.status}</p>
                </div>
              </div>
            ))}

            {history.length === 0 &&
            Object.keys(activeTranslations).length === 0 ? (
              <div className="flex-1 flex items-center justify-center py-24">
                <p className="text-white/20 text-sm">
                  Translations will appear here.
                </p>
              </div>
            ) : (
              <div className="flex flex-col">
                {history.length > 0 && (
                  <p
                    className="text-[10px] uppercase tracking-widest text-white/25"
                    style={{
                      marginLeft: "8px",
                      marginTop: "8px",
                      marginBottom: "8px",
                    }}
                  >
                    History
                  </p>
                )}
                <div className="flex flex-col gap-2.5">
                  {history.map((item) => (
                    <div
                      key={item.video_id}
                      className="bg-white/[0.03] p-3 flex gap-3.5 items-center cursor-pointer"
                      onClick={() => invoke("open_url", { url: item.url })}
                    >
                      <div className="w-28 h-16 overflow-hidden flex-shrink-0 bg-black/40">
                        <Thumbnail url={item.thumbnail_url} />
                      </div>
                      <div className="flex-1 min-w-0">
                        <p className="text-[13px] text-white/80 leading-snug line-clamp-2">
                          {item.title}
                        </p>
                        <p className="text-xs text-white/30 mt-1.5">
                          {[
                            item.author_name,
                            item.duration_secs !== undefined
                              ? formatDuration(item.duration_secs)
                              : undefined,
                            formatRelativeTime(item.created_at),
                          ]
                            .filter(Boolean)
                            .join(" · ")}
                        </p>
                      </div>
                      <div
                        className="flex gap-1 flex-shrink-0"
                        onClick={(e) => e.stopPropagation()}
                      >
                        <button
                          onClick={() => handleRedo(item)}
                          className="p-1.5 rounded-lg hover:bg-white/10 text-white/35 hover:text-white/70 cursor-pointer"
                          title="Redo translation"
                        >
                          <svg
                            className="w-3.5 h-3.5"
                            fill="none"
                            viewBox="0 0 24 24"
                            stroke="currentColor"
                            strokeWidth={2}
                          >
                            <path
                              strokeLinecap="round"
                              strokeLinejoin="round"
                              d="M4 4v5h.582m15.356 2A8.001 8.001 0 004.582 9m0 0H9m11 11v-5h-.581m0 0a8.003 8.003 0 01-15.357-2m15.357 2H15"
                            />
                          </svg>
                        </button>
                        <button
                          onClick={() => handleDelete(item.video_id)}
                          className="p-1.5 rounded-lg hover:bg-white/10 text-white/35 hover:text-red-400/70 cursor-pointer"
                          title="Delete translation"
                          style={{ marginRight: "6px" }}
                        >
                          <svg
                            className="w-3.5 h-3.5"
                            fill="none"
                            viewBox="0 0 24 24"
                            stroke="currentColor"
                            strokeWidth={2}
                          >
                            <path
                              strokeLinecap="round"
                              strokeLinejoin="round"
                              d="M19 7l-.867 12.142A2 2 0 0116.138 21H7.862a2 2 0 01-1.995-1.858L5 7m5 4v6m4-6v6m1-10V4a1 1 0 00-1-1h-4a1 1 0 00-1 1v3M4 7h16"
                            />
                          </svg>
                        </button>
                      </div>
                    </div>
                  ))}
                </div>
              </div>
            )}
          </div>
        </div>
      )}
    </div>
  );
}

export default App;

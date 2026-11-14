export const STRATEGY_LABELS: Record<string, string> = {
  socratic: "Gợi mở từng bước",
  coach: "Luyện và sửa trực tiếp",
};

export const SUBJECT_HINTS: Record<string, string> = {
  math: "Hỏi về một bài Toán, dán đề hoặc lời giải…",
  physics: "Hỏi về một bài Vật lý, dán đề hoặc công thức…",
  chemistry: "Hỏi về một bài Hóa học, dán phương trình…",
  english: "Dán đoạn văn hoặc hỏi về ngữ pháp, từ vựng…",
};

export const SUBJECT_SUGGESTIONS: Record<string, string[]> = {
  math: [
    "Giải thích đạo hàm bằng một ví dụ thực tế",
    "Kiểm tra lời giải phương trình này từng bước",
    "Cho mình 3 bài tập về bất phương trình",
  ],
  physics: [
    "Giải thích định luật Ohm bằng ví dụ đời thường",
    "Đổi 72 km/h sang m/s và giải thích cách làm",
    "Tóm tắt công thức chương dao động cơ",
  ],
  chemistry: [
    "Cân bằng phương trình Fe + O2 → Fe3O4",
    "Tính khối lượng mol của H2SO4",
    "Giải thích liên kết ion khác liên kết cộng hóa trị thế nào",
  ],
  english: [
    "Sửa đoạn văn tiếng Anh này và giữ giọng văn của mình",
    "Phân biệt thì hiện tại hoàn thành và quá khứ đơn",
    "Luyện 5 câu viết lại câu cho kỳ thi",
  ],
};

export const PROVIDER_LABELS: Record<string, string> = {
  groq: "Groq",
  gemini: "Google Gemini",
  openrouter: "OpenRouter",
  openai: "OpenAI",
  ollama: "Ollama (local)",
  custom: "Custom endpoint",
};

export const SUBJECT_LABELS: Record<string, string> = {
  math: "Toán",
  physics: "Vật lý",
  chemistry: "Hóa học",
  english: "Tiếng Anh",
};

export function subjectLabel(id: string): string {
  return SUBJECT_LABELS[id] ?? id;
}

export function subjectName(subjects: { id: string; name: string }[], id: string | null): string {
  if (!id) return "Buổi học";
  return subjects.find((subject) => subject.id === id)?.name ?? id;
}

export function subjectTone(id: string): string {
  return `tone-${id}`;
}

export function uid(): string {
  if (typeof crypto !== "undefined" && "randomUUID" in crypto) {
    return crypto.randomUUID();
  }
  return Math.random().toString(36).slice(2, 10);
}

export function relativeTime(value: string): string {
  let iso = value.replace(" ", "T");
  if (!/[zZ]|[+-]\d{2}:?\d{2}$/.test(iso)) iso += "Z";
  const date = new Date(iso);
  if (Number.isNaN(date.getTime())) return value;

  const diff = Date.now() - date.getTime();
  const minutes = Math.floor(diff / 60_000);
  if (minutes < 1) return "vừa xong";
  if (minutes < 60) return `${minutes} phút trước`;

  const now = new Date();
  if (date.toDateString() === now.toDateString()) {
    return `${Math.floor(minutes / 60)} giờ trước`;
  }
  const yesterday = new Date(now);
  yesterday.setDate(now.getDate() - 1);
  if (date.toDateString() === yesterday.toDateString()) return "hôm qua";
  return date.toLocaleDateString("vi-VN", { day: "2-digit", month: "2-digit" });
}

export function splitArgs(value: string): string[] {
  const args: string[] = [];
  let current = "";
  let quote: '"' | "'" | null = null;
  let started = false;

  for (const char of value) {
    if (quote) {
      if (char === quote) {
        quote = null;
      } else {
        current += char;
      }
      continue;
    }
    if (char === '"' || char === "'") {
      quote = char;
      started = true;
      continue;
    }
    if (/\s/.test(char)) {
      if (started || current) {
        args.push(current);
        current = "";
        started = false;
      }
      continue;
    }
    current += char;
    started = true;
  }
  if (started || current) args.push(current);
  return args;
}

export function parseEnv(value: string): Record<string, string> {
  const env: Record<string, string> = {};
  for (const rawLine of value.split("\n")) {
    const line = rawLine.trim();
    if (!line || line.startsWith("#")) continue;
    const index = line.indexOf("=");
    if (index <= 0) {
      throw new Error(`Dòng env không hợp lệ: ${line}`);
    }
    env[line.slice(0, index).trim()] = line.slice(index + 1).trim();
  }
  return env;
}

export function truncate(value: string, length: number): string {
  if (value.length <= length) return value;
  return `${value.slice(0, length - 1)}…`;
}

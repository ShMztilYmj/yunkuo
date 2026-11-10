import type { ReactNode } from "react";

/**
 * SKILL.md 的极简渲染：只处理 frontmatter、标题、列表、代码块、引用和行内强调。
 * 不引第三方 Markdown 库，也不用 dangerouslySetInnerHTML——能力库里的内容来自
 * 用户本地文件夹，按纯文本处理最稳。
 */
export interface SkillMeta {
  name?: string;
  description?: string;
}

export function splitFrontMatter(source: string): {
  meta: SkillMeta;
  body: string;
} {
  const text = source.replace(/\r\n/g, "\n");
  const match = /^---\n([\s\S]*?)\n---\n?/.exec(text);
  if (!match) return { meta: {}, body: text };
  const meta: SkillMeta = {};
  for (const line of match[1].split("\n")) {
    const at = line.indexOf(":");
    if (at <= 0) continue;
    const key = line.slice(0, at).trim().toLowerCase();
    const value = line
      .slice(at + 1)
      .trim()
      .replace(/^["']|["']$/g, "");
    if (!value) continue;
    if (key === "name") meta.name = value;
    if (key === "description") meta.description = value;
  }
  return { meta, body: text.slice(match[0].length) };
}

function inline(text: string): ReactNode[] {
  const nodes: ReactNode[] = [];
  const pattern = /(`[^`]+`|\*\*[^*]+\*\*)/g;
  let last = 0;
  let key = 0;
  for (const match of text.matchAll(pattern)) {
    const start = match.index ?? 0;
    if (start > last) nodes.push(text.slice(last, start));
    const token = match[0];
    if (token.startsWith("`")) {
      nodes.push(<code key={`c${key++}`}>{token.slice(1, -1)}</code>);
    } else {
      nodes.push(<strong key={`b${key++}`}>{token.slice(2, -2)}</strong>);
    }
    last = start + token.length;
  }
  if (last < text.length) nodes.push(text.slice(last));
  return nodes;
}

export default function SkillBody({ source }: { source: string }) {
  const lines = source.split("\n");
  const blocks: ReactNode[] = [];
  let i = 0;
  let key = 0;

  while (i < lines.length) {
    const line = lines[i];

    if (/^```/.test(line)) {
      const body: string[] = [];
      i += 1;
      while (i < lines.length && !/^```/.test(lines[i])) {
        body.push(lines[i]);
        i += 1;
      }
      i += 1;
      blocks.push(
        <pre className="skill-code" key={`p${key++}`}>
          <code>{body.join("\n")}</code>
        </pre>,
      );
      continue;
    }

    const heading = /^(#{1,6})\s+(.*)$/.exec(line);
    if (heading) {
      const level = Math.min(heading[1].length, 3);
      blocks.push(
        <p className={`skill-heading is-h${level}`} key={`h${key++}`}>
          {inline(heading[2])}
        </p>,
      );
      i += 1;
      continue;
    }

    if (/^\s*([-*]|\d+\.)\s+/.test(line)) {
      const ordered = /^\s*\d+\.\s+/.test(line);
      const items: ReactNode[] = [];
      while (i < lines.length && /^\s*([-*]|\d+\.)\s+/.test(lines[i])) {
        const text = lines[i].replace(/^\s*([-*]|\d+\.)\s+/, "");
        items.push(<li key={`i${key++}`}>{inline(text)}</li>);
        i += 1;
      }
      blocks.push(
        ordered ? (
          <ol className="skill-list" key={`l${key++}`}>
            {items}
          </ol>
        ) : (
          <ul className="skill-list" key={`l${key++}`}>
            {items}
          </ul>
        ),
      );
      continue;
    }

    if (/^>\s?/.test(line)) {
      const quoted: string[] = [];
      while (i < lines.length && /^>\s?/.test(lines[i])) {
        quoted.push(lines[i].replace(/^>\s?/, ""));
        i += 1;
      }
      blocks.push(
        <blockquote className="skill-quote" key={`q${key++}`}>
          {inline(quoted.join(" "))}
        </blockquote>,
      );
      continue;
    }

    if (!line.trim()) {
      i += 1;
      continue;
    }

    const paragraph: string[] = [];
    while (
      i < lines.length &&
      lines[i].trim() &&
      !/^(#{1,6})\s|^```|^\s*([-*]|\d+\.)\s|^>/.test(lines[i])
    ) {
      paragraph.push(lines[i].trim());
      i += 1;
    }
    blocks.push(
      <p className="skill-text" key={`t${key++}`}>
        {inline(paragraph.join(" "))}
      </p>,
    );
  }

  return <div className="skill-body">{blocks}</div>;
}

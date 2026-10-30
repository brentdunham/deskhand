import { Fragment, type ReactNode } from "react";

// A small Markdown renderer for model answers. It produces React elements
// only, never HTML strings: answers can be shaped by untrusted page content,
// and HTML injected into this webview could reach DeskHand's IPC commands.
// Links are shown as text, not anchors, so nothing can navigate the webview.

type Block = { kind: "code"; lang: string; code: string } | { kind: "text"; text: string };

/** Splits on fenced code blocks. An unclosed fence (mid-stream) runs to the end. */
export function splitBlocks(md: string): Block[] {
  const blocks: Block[] = [];
  const lines = md.split("\n");
  let text: string[] = [];
  let code: string[] | null = null;
  let lang = "";
  const flushText = () => {
    if (text.join("").trim()) blocks.push({ kind: "text", text: text.join("\n") });
    text = [];
  };
  for (const line of lines) {
    const fence = line.match(/^\s*```(.*)$/);
    if (fence && code === null) {
      flushText();
      code = [];
      lang = fence[1].trim();
    } else if (fence && code !== null) {
      blocks.push({ kind: "code", lang, code: code.join("\n") });
      code = null;
    } else if (code !== null) {
      code.push(line);
    } else {
      text.push(line);
    }
  }
  if (code !== null) blocks.push({ kind: "code", lang, code: code.join("\n") });
  flushText();
  return blocks;
}

/** `code` and **bold** inside a line. */
function inline(text: string): ReactNode[] {
  return text.split(/(`[^`\n]+`|\*\*[^*\n]+\*\*)/g).map((part, i) => {
    if (part.startsWith("`") && part.endsWith("`") && part.length > 2) {
      return <code key={i}>{part.slice(1, -1)}</code>;
    }
    if (part.startsWith("**") && part.endsWith("**") && part.length > 4) {
      return <strong key={i}>{part.slice(2, -2)}</strong>;
    }
    return <Fragment key={i}>{part}</Fragment>;
  });
}

function TextBlock({ text }: { text: string }) {
  const paragraphs = text.split(/\n\s*\n/).filter((p) => p.trim());
  return (
    <>
      {paragraphs.map((p, i) => {
        const lines = p.split("\n").filter((l) => l.trim());
        const bullets = lines.every((l) => /^\s*([-*]|\d+[.)])\s+/.test(l));
        if (bullets) {
          const ordered = /^\s*\d/.test(lines[0]);
          const items = lines.map((l, j) => (
            <li key={j}>{inline(l.replace(/^\s*([-*]|\d+[.)])\s+/, ""))}</li>
          ));
          return ordered ? <ol key={i}>{items}</ol> : <ul key={i}>{items}</ul>;
        }
        const heading = lines.length === 1 && lines[0].match(/^#{1,4}\s+(.*)$/);
        if (heading) {
          return (
            <p key={i} className="md-heading">
              {inline(heading[1])}
            </p>
          );
        }
        return (
          <p key={i}>
            {lines.map((l, j) => (
              <Fragment key={j}>
                {j > 0 && <br />}
                {inline(l)}
              </Fragment>
            ))}
          </p>
        );
      })}
    </>
  );
}

function CodeBlock({ lang, code }: { lang: string; code: string }) {
  return (
    <div className="md-code">
      <div className="md-code-head">
        <span>{lang || "code"}</span>
        <button type="button" onClick={() => void navigator.clipboard.writeText(code)}>
          Copy
        </button>
      </div>
      <pre>
        <code>{code}</code>
      </pre>
    </div>
  );
}

export default function Markdown({ text }: { text: string }) {
  return (
    <div className="md">
      {splitBlocks(text).map((b, i) =>
        b.kind === "code" ? (
          <CodeBlock key={i} lang={b.lang} code={b.code} />
        ) : (
          <TextBlock key={i} text={b.text} />
        ),
      )}
    </div>
  );
}

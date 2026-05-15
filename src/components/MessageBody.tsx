// Renders a message body the way claude-code and codex output —
// GFM markdown with code blocks, lists, headings, inline code, links.
// The bundled CLI streams whatever the agent typed, which in claude/
// codex is markdown by convention. Plain-text rendering loses that
// structure (lists collapse into one line, code becomes prose, etc.).
//
// Styling tuned for a chat-log feel: tight line-height, generous
// monospace blocks, accent green for inline code so it pops against
// the carbon background. Lists keep their bullets and indentation.

import ReactMarkdown from "react-markdown";
import remarkBreaks from "remark-breaks";
import remarkGfm from "remark-gfm";

import { openUrl } from "@tauri-apps/plugin-opener";

export function MessageBody({ text }: { text: string }) {
  return (
    <div className="prose-message">
      <ReactMarkdown
        remarkPlugins={[remarkGfm, remarkBreaks]}
        components={{
          // Route clicks through Tauri's shell-opener so the URL
          // lands in the user's default browser. preventDefault stays
          // because a bare anchor navigation would tear the WKWebView
          // off the SPA.
          a: ({ children, href }) => (
            <a
              href={href ?? "#"}
              onClick={(e) => {
                e.preventDefault();
                if (href) void openUrl(href).catch(() => {});
              }}
              className="text-accent underline decoration-accent/40 underline-offset-2 hover:decoration-accent"
            >
              {children}
            </a>
          ),
          code: ({ className, children }) => {
            const isBlock = (className ?? "").startsWith("language-");
            if (isBlock) {
              return (
                <code
                  className={`block whitespace-pre overflow-x-auto rounded-md border border-line bg-bg px-3 py-2 font-mono text-[12px] leading-relaxed text-fg ${className ?? ""}`}
                >
                  {children}
                </code>
              );
            }
            return (
              <code className="rounded bg-raised px-1 py-px font-mono text-[12px] text-accent">
                {children}
              </code>
            );
          },
          pre: ({ children }) => <pre className="my-2">{children}</pre>,
          ul: ({ children }) => (
            <ul className="my-1 ml-5 list-disc space-y-0.5">{children}</ul>
          ),
          ol: ({ children }) => (
            <ol className="my-1 ml-5 list-decimal space-y-0.5">{children}</ol>
          ),
          li: ({ children }) => <li className="leading-snug">{children}</li>,
          h1: ({ children }) => (
            <h1 className="mt-3 mb-1.5 text-[14px] font-semibold text-fg">
              {children}
            </h1>
          ),
          h2: ({ children }) => (
            <h2 className="mt-3 mb-1 text-[13px] font-semibold text-fg">
              {children}
            </h2>
          ),
          h3: ({ children }) => (
            <h3 className="mt-2 mb-1 text-[13px] font-semibold text-fg-2">
              {children}
            </h3>
          ),
          p: ({ children }) => (
            <p className="my-1 leading-relaxed">{children}</p>
          ),
          blockquote: ({ children }) => (
            <blockquote className="my-2 border-l-2 border-line pl-3 text-fg-2">
              {children}
            </blockquote>
          ),
          hr: () => <hr className="my-3 border-line" />,
          strong: ({ children }) => (
            <strong className="font-semibold text-fg">{children}</strong>
          ),
          em: ({ children }) => <em className="italic text-fg">{children}</em>,
          table: ({ children }) => (
            <div className="my-2 overflow-x-auto">
              <table className="border-collapse text-[12px]">{children}</table>
            </div>
          ),
          th: ({ children }) => (
            <th className="border border-line bg-raised px-2 py-1 text-left font-semibold text-fg">
              {children}
            </th>
          ),
          td: ({ children }) => (
            <td className="border border-line px-2 py-1 align-top text-fg-2">
              {children}
            </td>
          ),
        }}
      >
        {text}
      </ReactMarkdown>
    </div>
  );
}

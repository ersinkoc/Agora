- If you write your own agent manifest: every `[[typed]]` rule now needs a `regex` that must match (the rule's own `regex`, or one
  in `all`); a manifest file with a `[[typed]]` rule made only of phrases is not loaded. Write the regex so it needs the footer whole
  on one row right under the input box, so a helper's prompt holding a footer word the box breaks in its middle cannot stand in for
  the real footer. The manifests Agora ships already do this (#1150, ADR-0057).

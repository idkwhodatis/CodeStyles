# CodeStyles

Preview **bold**, _emphasis_, ~~strikethrough~~ and `inline code` in a deliberately long paragraph so line wrapping is easy to compare between the saved settings and the draft settings.

## Links and lists

Visit [dprint](https://dprint.dev/ "Formatter") or use a [reference][guide].

*   Keep the source unchanged
    *   Compare indentation and nesting
*   Preview the result

1.  Select a language
2.  Adjust a setting

- [x] Saved settings
- [ ] Draft settings

>   A block quote can contain **emphasis** and a long sentence that wraps differently at narrower line widths.
>
>   A second paragraph.

|Name|Enabled|Width|
|:--|:--:|--:|
|Saved|true|80|
|Draft|false|100|

---

<!-- A comment stays beside the code examples. -->

```ts
const settings={name:'CodeStyles',enabled:true};
```

```json
{"labels":["saved","draft"],"width":80}
```

```toml
name="CodeStyles"
enabled=true
```

[guide]: https://dprint.dev/config/ "Configuration"

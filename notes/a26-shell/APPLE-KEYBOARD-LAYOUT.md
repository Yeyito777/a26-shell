# Moon English (US) phone keyboard layout

Moon targets the familiar Apple iPhone English (US) portrait arrangement while retaining Moon's own visual language.

Apple does not publish fixed key rectangles for the system keyboard. UIKit exposes semantic keyboard types and Apple documents behaviors, but iOS chooses geometry according to device, locale, enabled keyboards, safe-area insets, and text-input traits. Moon therefore uses the standard iPhone arrangement and proportional geometry rather than claiming proprietary pixel measurements.

## Reference behavior

Primary Apple references:

- Apple Human Interface Guidelines: Keyboards
  - https://developer.apple.com/design/human-interface-guidelines/keyboards
- `UIKeyboardType`
  - https://developer.apple.com/documentation/uikit/uikeyboardtype
- `UIReturnKeyType`
  - https://developer.apple.com/documentation/uikit/uireturnkeytype
- iPhone User Guide: Type with the onscreen keyboard
  - https://support.apple.com/guide/iphone/type-with-the-onscreen-keyboard-iph3c50f96e/ios

## Moon geometry at 1080 x 2340

- Keyboard window: 1080 x 820, rooted at y=1520.
- Key area: 640 pixels.
- Moon close-gesture/safe area: final 180 pixels.
- Four key rows begin at local y=18, 166, 314, and 462.
- Letter-row key heights: 132 pixels.
- Bottom-row key height: 160 pixels.
- Inter-key gap: 10 pixels.
- QWERTY row margin: 16 pixels.
- ASDF row margin: 64 pixels, producing the standard centered stagger.
- Shift/ZXCVBNM/Delete row margin: 16 pixels, with wider outside controls.

Horizontal geometry and margins scale proportionally on other widths.

## English (US) layers

Letters:

```text
q w e r t y u i o p
  a s d f g h j k l
Shift z x c v b n m Delete
123          Space          Done/Search
```

The URL/search field uses the familiar Safari-style contextual bottom row:

```text
123          Space          .          Go
```

Numbers and punctuation:

```text
1 2 3 4 5 6 7 8 9 0
- / : ; ( ) $ & @ "
#+=  .  ,  ?  !  '  Delete
ABC          Space          Done/Search/Go
```

Additional symbols:

```text
[ ] { } # % ^ * + =
_ \ | ~ < > $ & @
123  .  ,  ?  !  '  Delete
ABC          Space          Done/Search/Go
```

Number-purpose fields receive a dedicated 3-by-4 number pad.

## Deliberate limits

This change targets physical layout and muscle memory. Predictive suggestions, QuickPath, long-press alternates, space-bar trackpad mode, emoji/language switching, dictation, haptics, and locale-specific layers are separate features. Moon retains one-shot Shift, password non-retention, XTEST focus provenance checks, and the global bottom-edge app-close gesture.

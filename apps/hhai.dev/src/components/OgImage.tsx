// Fallback fonts and Emoji are dynamically loaded
// from Google Fonts and CDNs in this demo.

// You can also return a function component in the playground.
export default function OgImage ()  {
    function Label({ children }) {
      return <label style={{
        fontSize: 15,
        fontWeight: 600,
        textTransform: 'uppercase',
        letterSpacing: 1,
        margin: '25px 0 10px',
        color: 'gray',
      }}>
        {children}
      </label>
    }
  
    return (
      <div
        style={{
          display: 'flex',
          flexDirection: 'column',
          height: '100%',
          width: '100%',
          padding: '10px 20px',
          justifyContent: 'center',
          fontFamily: 'Inter, "Material Icons"',
          fontSize: 28,
          backgroundColor: 'white',
        }}
        >
        <Label>Language & Font subsets</Label>
        <div>
          Hello! 你好! 안녕! こんにちは! Χαίρετε! Hallå!
        </div>
        <Label>Emoji</Label>
        <div>
          👋 😄 🎉 🎄 🦋
        </div>
        <Label>Icon font</Label>
        <div>
            &#xe766; &#xeb9b; &#xf089;
        </div>
        <Label>Lang attribute</Label>
        <div style={{ display: 'flex' }}>
          <span lang="ja-JP">
            骨茶
          </span>/
          <span lang="zh-CN">
            骨茶
          </span>/
          <span lang="zh-TW">
            骨茶
          </span>/
          <span lang="zh-HK">
            骨茶
          </span>
        </div>
      </div>
    )
  }  
  
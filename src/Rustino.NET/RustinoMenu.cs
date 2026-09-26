using System.Text.Json;
using System.Text.Json.Serialization;

namespace Rustino.NET;

public partial class RustinoMenu
{
    private readonly List<MenuItemDef> _items = new();

    // iconPath: image shown next to the label (scaled to the menu's icon size)
    public RustinoMenu AddItem(string id, string label, string? accelerator = null, bool enabled = true, string? iconPath = null)
    {
        _items.Add(new MenuItemDef { Type = "normal", Id = id, Label = label, Accelerator = accelerator, Enabled = enabled, Icon = iconPath });
        return this;
    }

    // Clicking toggles the check mark and raises MenuItemClicked and MenuItemCheckedChanged.
    public RustinoMenu AddCheckItem(string id, string label, bool isChecked = false, bool enabled = true, string? accelerator = null)
    {
        _items.Add(new MenuItemDef { Type = "check", Id = id, Label = label, Checked = isChecked, Enabled = enabled, Accelerator = accelerator });
        return this;
    }

    public RustinoMenu AddSeparator()
    {
        _items.Add(new MenuItemDef { Type = "separator" });
        return this;
    }

    // Native OS action (Copy, Paste, Quit, ...) that fires no MenuItemClicked.
    // On macOS, Cmd+C/V/X/A reach the webview only through these Edit items.
    // Some items are omitted on Linux or do nothing on Windows (see README).
    public RustinoMenu AddPredefinedItem(PredefinedMenuItem item, string? label = null)
    {
        _items.Add(new MenuItemDef { Type = "predefined", Item = JsonNamingPolicy.SnakeCaseLower.ConvertName(item.ToString()), Label = label });
        return this;
    }

    // macOS application menu (shown with the app name), replacing the standard one; ignored on Windows/Linux.
    public RustinoMenu AddAppMenu(Action<RustinoMenu> build)
    {
        var sub = new RustinoMenu();
        build(sub);
        _items.Add(new MenuItemDef { Type = "app_menu", Items = sub._items });
        return this;
    }

    public RustinoMenu AddSubmenu(string label, Action<RustinoMenu> build, bool enabled = true)
    {
        return AddSubmenuWithRole(label, build, enabled, role: null);
    }

    // macOS Window menu: the system appends the open windows; a normal submenu on Windows/Linux.
    public RustinoMenu AddWindowMenu(string label, Action<RustinoMenu> build)
    {
        return AddSubmenuWithRole(label, build, enabled: true, role: "window");
    }

    // macOS Help menu: the system adds a search field; a normal submenu on Windows/Linux.
    public RustinoMenu AddHelpMenu(string label, Action<RustinoMenu> build)
    {
        return AddSubmenuWithRole(label, build, enabled: true, role: "help");
    }

    private RustinoMenu AddSubmenuWithRole(string label, Action<RustinoMenu> build, bool enabled, string? role)
    {
        var sub = new RustinoMenu();
        build(sub);
        _items.Add(new MenuItemDef { Type = "submenu", Label = label, Enabled = enabled, Role = role, Items = sub._items });
        return this;
    }

    internal string ToJson()
    {
        return JsonSerializer.Serialize(_items, MenuJsonContext.Default.ListMenuItemDef);
    }

    private class MenuItemDef
    {
        [JsonPropertyName("type")]
        public string Type { get; set; } = "";

        [JsonPropertyName("id")]
        [JsonIgnore(Condition = JsonIgnoreCondition.WhenWritingNull)]
        public string? Id { get; set; }

        [JsonPropertyName("item")]
        [JsonIgnore(Condition = JsonIgnoreCondition.WhenWritingNull)]
        public string? Item { get; set; }

        [JsonPropertyName("label")]
        [JsonIgnore(Condition = JsonIgnoreCondition.WhenWritingNull)]
        public string? Label { get; set; }

        [JsonPropertyName("accelerator")]
        [JsonIgnore(Condition = JsonIgnoreCondition.WhenWritingNull)]
        public string? Accelerator { get; set; }

        [JsonPropertyName("enabled")]
        [JsonIgnore(Condition = JsonIgnoreCondition.WhenWritingNull)]
        public bool? Enabled { get; set; }

        [JsonPropertyName("checked")]
        [JsonIgnore(Condition = JsonIgnoreCondition.WhenWritingNull)]
        public bool? Checked { get; set; }

        [JsonPropertyName("icon")]
        [JsonIgnore(Condition = JsonIgnoreCondition.WhenWritingNull)]
        public string? Icon { get; set; }

        [JsonPropertyName("role")]
        [JsonIgnore(Condition = JsonIgnoreCondition.WhenWritingNull)]
        public string? Role { get; set; }

        [JsonPropertyName("items")]
        [JsonIgnore(Condition = JsonIgnoreCondition.WhenWritingNull)]
        public List<MenuItemDef>? Items { get; set; }
    }

    [JsonSerializable(typeof(List<MenuItemDef>))]
    private partial class MenuJsonContext : JsonSerializerContext;
}

public enum PredefinedMenuItem
{
    About,
    Undo,
    Redo,
    Cut,
    Copy,
    Paste,
    SelectAll,
    Minimize,
    Maximize,
    Fullscreen,
    Hide,
    HideOthers,
    ShowAll,
    CloseWindow,
    Quit,
    Services,
    BringAllToFront,
}

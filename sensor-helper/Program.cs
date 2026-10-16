using LibreHardwareMonitor.Hardware;
using System.Globalization;

// Read-only telemetry bridge. This intentionally exposes no HTTP server and
// does not invoke any sensor/control setters.
try
{
    var computer = new Computer
    {
        IsCpuEnabled = true,
        IsGpuEnabled = true,
        IsMemoryEnabled = true,
        IsMotherboardEnabled = true,
        IsControllerEnabled = true,
        IsStorageEnabled = true,
    };

    computer.Open();
    try
    {
        foreach (var hardware in computer.Hardware)
            EmitHardware(hardware);
    }
    finally
    {
        computer.Close();
    }

    return 0;
}
catch (Exception ex)
{
    // Keep stdout machine-readable; diagnostics are sent to stderr.
    Console.Error.WriteLine($"Sensor provider error: {ex.GetType().Name}: {ex.Message}");
    return 1;
}

static void EmitHardware(IHardware hardware)
{
    hardware.Update();

    var parentType = hardware.Parent is null ? string.Empty : hardware.Parent.HardwareType.ToString();
    var parentName = hardware.Parent?.Name ?? string.Empty;
    foreach (var sensor in hardware.Sensors)
    {
        if (sensor.Value is not { } value || !float.IsFinite(value) || value < 0)
            continue;
        if ((sensor.SensorType is SensorType.Temperature or SensorType.Voltage or SensorType.Power)
            && value == 0)
            continue;

        var fields = new[]
        {
            hardware.HardwareType.ToString(),
            hardware.Name,
            parentType,
            parentName,
            sensor.SensorType.ToString(),
            sensor.Name,
            value.ToString("G9", CultureInfo.InvariantCulture),
            UnitFor(sensor.SensorType),
        };
        Console.WriteLine(string.Join('\t', fields.Select(EscapeTsv)));
    }

    foreach (var child in hardware.SubHardware)
        EmitHardware(child);
}

static string UnitFor(SensorType type) => type switch
{
    SensorType.Temperature => "°C",
    SensorType.Fan => "RPM",
    SensorType.Voltage => "V",
    SensorType.Power => "W",
    SensorType.Clock => "MHz",
    SensorType.Load or SensorType.Control or SensorType.Level => "%",
    SensorType.Factor => "x",
    SensorType.Data => "GB",
    SensorType.SmallData => "MB",
    SensorType.Throughput => "B/s",
    _ => string.Empty,
};

static string EscapeTsv(string value) => value
    .Replace("\\", "\\\\", StringComparison.Ordinal)
    .Replace("\t", "\\t", StringComparison.Ordinal)
    .Replace("\r", "\\r", StringComparison.Ordinal)
    .Replace("\n", "\\n", StringComparison.Ordinal);

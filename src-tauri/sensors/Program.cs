// CPU-only, local helper for Open Dock. No network, tuning or installer APIs.
using System;
using System.Collections.Generic;
using System.Security.Principal;
using System.Web.Script.Serialization;
using LibreHardwareMonitor.Hardware;
using LibreHardwareMonitor.PawnIo;

namespace OpenDock.Sensors
{
    internal static class Program
    {
        private static Computer computer;
        private static readonly JavaScriptSerializer Serializer = new JavaScriptSerializer();
        private static long sequence;

        private static Dictionary<string, object> Sample()
        {
            var result = new Dictionary<string, object>
            {
                { "schema", 1 },
                { "sequence", sequence },
                { "at", DateTimeOffset.UtcNow.ToUnixTimeMilliseconds() },
                { "celsius", null },
                { "source", "lhm-direct-package" },
                { "status", "sensor-unavailable" }
            };
            if (!PawnIo.IsInstalled || PawnIo.Version < new Version(2, 2))
            {
                result["status"] = "driver-required";
                return result;
            }
            using (var identity = WindowsIdentity.GetCurrent())
            {
                if (!new WindowsPrincipal(identity).IsInRole(WindowsBuiltInRole.Administrator))
                {
                    result["status"] = "admin-required";
                    return result;
                }
            }
            if (computer == null)
            {
                computer = new Computer
                {
                    IsCpuEnabled = true,
                    IsGpuEnabled = false,
                    IsMemoryEnabled = false,
                    IsMotherboardEnabled = false,
                    IsControllerEnabled = false,
                    IsNetworkEnabled = false,
                    IsStorageEnabled = false,
                    IsBatteryEnabled = false,
                    IsPowerMonitorEnabled = false
                };
                computer.Open();
            }
            var candidates = new List<SensorCandidate>();
            foreach (IHardware hardware in computer.Hardware)
            {
                if (hardware.HardwareType != HardwareType.Cpu) continue;
                hardware.Update();
                foreach (ISensor sensor in hardware.Sensors)
                {
                    if (sensor.SensorType == SensorType.Temperature && sensor.Value.HasValue)
                    {
                        candidates.Add(new SensorCandidate
                        {
                            IsCpu = true,
                            HardwareId = hardware.Identifier.ToString(),
                            HardwareName = hardware.Name,
                            SensorId = sensor.Identifier.ToString(),
                            SensorName = sensor.Name,
                            Celsius = sensor.Value.Value
                        });
                    }
                }
            }
            var reading = TemperatureSelection.Select(candidates);
            if (reading != null)
            {
                result["status"] = "ready";
                result["celsius"] = reading.Celsius;
                result["hardware"] = reading.HardwareName;
                result["sensor"] = reading.SensorName;
                result["sensorId"] = reading.SensorId;
            }
            result["at"] = DateTimeOffset.UtcNow.ToUnixTimeMilliseconds();
            return result;
        }

        public static int Main()
        {
            // Wait before opening hardware: the native parent attaches its job
            // object before sending the first command. EOF means parent closed.
            try
            {
                string command;
                while ((command = Console.ReadLine()) != null)
                {
                    if (command != "sample") return 2;
                    sequence++;
                    try
                    {
                        Console.WriteLine(Serializer.Serialize(Sample()));
                    }
                    catch (Exception error)
                    {
                        Console.Error.WriteLine("[cpu-sensor] " + error.GetType().Name + ": " + error.Message);
                        Console.WriteLine(Serializer.Serialize(new
                        {
                            schema = 1,
                            sequence = sequence,
                            at = DateTimeOffset.UtcNow.ToUnixTimeMilliseconds(),
                            celsius = (float?)null,
                            source = "lhm-direct-package",
                            status = "sensor-error"
                        }));
                        return 1;
                    }
                }
                return 0;
            }
            finally
            {
                if (computer != null) computer.Close();
            }
        }
    }
}

using System;
using System.Collections.Generic;
using System.Linq;

namespace OpenDock.Sensors
{
    internal sealed class SensorCandidate
    {
        public bool IsCpu;
        public string HardwareId;
        public string HardwareName;
        public string SensorId;
        public string SensorName;
        public float Celsius;
    }

    internal static class TemperatureSelection
    {
        private static int Rank(string name)
        {
            switch ((name ?? "").Trim().ToLowerInvariant())
            {
                case "cpu package":
                case "core (tctl/tdie)":
                case "core (tdie)":
                    return 1;
                default:
                    return 0;
            }
        }

        public static SensorCandidate Select(IEnumerable<SensorCandidate> sensors)
        {
            // Only actual CPU-package/die sensors. A CCD, motherboard zone or
            // control temperature with a vendor offset is a different reading.
            return sensors
                .Where(s => s.IsCpu && Rank(s.SensorName) > 0
                    && !float.IsNaN(s.Celsius) && !float.IsInfinity(s.Celsius)
                    && s.Celsius > 0 && s.Celsius < 125)
                .GroupBy(s => s.HardwareId)
                .Select(group => group.OrderBy(s => Rank(s.SensorName)).First())
                .OrderByDescending(s => s.Celsius)
                .FirstOrDefault();
        }
    }
}

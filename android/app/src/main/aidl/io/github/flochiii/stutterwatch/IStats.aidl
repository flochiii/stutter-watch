package io.github.flochiii.stutterwatch;
import android.os.ParcelFileDescriptor;

interface IStats {
    void attach(IBinder client) = 0;
    String sample(String target, boolean frames, boolean gpu) = 1;
    String startTrace(in ParcelFileDescriptor output, boolean full) = 2;
    String stopTrace() = 3;
    String traceStatus() = 4;
    void destroy() = 16777114;
}

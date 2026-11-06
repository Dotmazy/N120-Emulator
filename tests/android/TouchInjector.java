import android.os.SystemClock;
import android.view.InputEvent;
import android.view.MotionEvent;
import android.view.InputDevice;
import java.lang.reflect.Method;

/** ADB shell test helper; never packaged in the app. Injects genuine multi-pointer touch events. */
public class TouchInjector {
    private static Object manager;
    private static Method inject;
    private static long start;
    private static void event(int action, int count, float x1, float y1, float x2, float y2) throws Exception {
        MotionEvent.PointerProperties[] properties = new MotionEvent.PointerProperties[count];
        MotionEvent.PointerCoords[] coordinates = new MotionEvent.PointerCoords[count];
        for (int i=0; i<count; ++i) {
            properties[i] = new MotionEvent.PointerProperties();
            properties[i].id = i; properties[i].toolType = MotionEvent.TOOL_TYPE_FINGER;
            coordinates[i] = new MotionEvent.PointerCoords();
            coordinates[i].x = i==0 ? x1 : x2;
            coordinates[i].y = i==0 ? y1 : y2;
            coordinates[i].pressure = 1; coordinates[i].size = 0.1f;
        }
        MotionEvent event = MotionEvent.obtain(start,SystemClock.uptimeMillis(),action,count,
            properties,coordinates,0,0,1,1,0,0,InputDevice.SOURCE_TOUCHSCREEN,0);
        if (!((Boolean)inject.invoke(manager,event,2))) throw new RuntimeException("Touch injection rejected");
        event.recycle();
    }
    public static void main(String[] args) throws Exception {
        Class<?> type = Class.forName("android.hardware.input.InputManagerGlobal");
        manager = type.getMethod("getInstance").invoke(null);
        inject = type.getMethod("injectInputEvent",InputEvent.class,int.class);
        float x1=Float.parseFloat(args[0]),y1=Float.parseFloat(args[1]);
        float x2=Float.parseFloat(args[2]),y2=Float.parseFloat(args[3]);
        int duration=Integer.parseInt(args[4]);
        start=SystemClock.uptimeMillis();
        event(MotionEvent.ACTION_DOWN,1,x1,y1,x2,y2);
        event(MotionEvent.ACTION_POINTER_DOWN|(1<<MotionEvent.ACTION_POINTER_INDEX_SHIFT),2,x1,y1,x2,y2);
        Thread.sleep(duration);
        if (args.length>5 && args[5].equals("cancel")) event(MotionEvent.ACTION_CANCEL,2,x1,y1,x2,y2);
        else {
            event(MotionEvent.ACTION_POINTER_UP|(1<<MotionEvent.ACTION_POINTER_INDEX_SHIFT),2,x1,y1,x2,y2);
            event(MotionEvent.ACTION_UP,1,x1,y1,x2,y2);
        }
    }
}

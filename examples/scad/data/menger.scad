//author Gustavo Segura <gsegura96@openmailbox.org>
/* ToDo:
* Document the code
* Add license
* Optimize
*/
module MengerSponge(side = 270, order =2){
    difference(){
        cube([side,side,side],center=true);
        MengerSponge_aux1(side, order);
    }
    }
module MengerSponge_aux1(side, order){
    rotations=[
    [90,0,0],
    [0,90,0],
    [0,0,90],
    ];
    for(rotation=rotations){
        rotate(rotation)
    MengerSponge_aux2(side, order);
        }
}


 module MengerSponge_aux2(side, order){
     if(order>0){
        translate([0,-(side-(side/pow(3,order-1)))/2,0])
            for(i=[0:pow(3,order-1)-1]){
                translate([0,(side/pow(3,order-1))*i,0])
                    translate([-(side-(side/pow(3,order-1)))/2,0,0])
                        for(i=[0:pow(3,order-1)-1]){
                            translate([(side/pow(3,order-1))*i,0,0])
                                cube([(side/pow(3,order)),(side/pow(3,order)),side+1],center=true);
            }
        }
        MengerSponge_aux2(side, order-1);
     }
     else{
         echo("terminado");
     }
 }
side=270;
order=2;//Dont save with a number higher than 4, OpenSCAD will collapse when trying to render it.
MengerSponge(side, order);
